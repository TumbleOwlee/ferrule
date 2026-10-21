use ferrule_core::card::board::Slug;
use std::fmt::Write as _;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use clap::{Subcommand, ValueEnum};
use ferrule_core::card::board::{self, CardError, LogError, State};
use ferrule_core::card::vocab;
use ferrule_core::config::Config;
use ferrule_core::pr::publish;
use ferrule_core::project;
use ferrule_core::refusal::{Exit, Refusal};
use ferrule_core::store::{Board, Error as StoreError};
use ferrule_core::term::{self, Style};

use super::{exit, internal, refuse};

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum CardKind {
    /// A stage card, `<slug>.s<n>`.
    Stage,
    /// A run's parent card, `<slug>`.
    Parent,
}

/// What `board show` prints of a card.
struct ShowPart<'a> {
    fields: bool,
    log: bool,
    last: Option<usize>,
    event: Option<&'a str>,
}

impl ShowPart<'_> {
    /// `text`, a card as `board::show` renders it, cut to this part; `log`
    /// is its log as `(at, body)`, already cut to the part's event.
    fn cut(&self, text: &str, mut log: Vec<(String, String)>) -> String {
        let fields = text.split("---\n").nth(1).unwrap_or("");
        if let Some(n) = self.last {
            log.drain(..log.len().saturating_sub(n));
        }
        let mut out = String::new();
        if self.fields {
            let _ = write!(out, "---\n{fields}---\n");
        }
        if self.log {
            for (at, body) in log {
                let _ = writeln!(out, "{at} {body}");
            }
        }
        out
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum CardState {
    /// Not started.
    Open,
    /// Being worked on.
    Inprogress,
    /// Waiting for review or approval.
    Inreview,
    /// Finished.
    Done,
}

const EVENTS_AFTER_LONG_HELP: &str = "\
Output:
  <card> <at> <event> ...    One log line per match across the run's parent
                             card and all its stage cards, in the order
                             they were logged. Nothing when no line
                             matches.
Filters: --event <name> keeps that event's lines (`follow-up`, `spec-delta`,
`commit`, ...); --last <n> the latest n of them.
Refusals:
  events=refused reason=card hint=check the slug with `ferrule board list`
                             No such run (exit 1).
Exit codes: 0 ok, 1 refused, 4 internal error.
Examples:
  ferrule board events <slug> --event follow-up
  ferrule board events <slug> --event spec-delta --last 5
See: ferrule guide show board";

const BOARD_AFTER_LONG_HELP: &str = "\
Output:
  <state> <id>  <title>      One line per card, parents before their stages;
                             the title is the card's `title` field, if any.
                             Nothing when there is no board yet.
Filters: --state keeps the cards in that state (repeat for several);
--run <slug> keeps that run's parent and its stages. Both together keep the
cards that match both.
Exit codes: 0 ok, 4 internal error.
Examples:
  ferrule board list --state inprogress --state inreview
  ferrule board list --run <slug>
See: ferrule guide show board";

const SHOW_AFTER_LONG_HELP: &str = "\
Output:
  ---                        Markdown: frontmatter between `---` fences (id,
  id: <id>                   parent for a stage card, state, then the fields),
  state: <state>             then one `<UTC minute> <event> ...` line per log
  ---                        entry, oldest first.
Filters: --fields prints only the frontmatter; --log only the log lines.
--event <name> keeps the log lines of that event (`commit`, `gauntlet`,
`test-red`, ...), --last <n> the latest n of them; either one still prints
the frontmatter unless --log is given.
Examples:
  ferrule board show <slug>.s1 --log --last 5
  ferrule board show <slug>.s1 --event commit
Refusals:
  card-show=refused reason=card
                             No such card; check it with
                             `ferrule board list` (hint= says so)
Exit codes: 0 ok, 1 refused, 4 internal error.
See: ferrule guide show board";

const FIELD_AFTER_LONG_HELP: &str = "\
Output:
  <value>                    The value on one line. Nothing, with exit 0, when
                             the card, the field or the log key is absent.
Fields:
  <name>                     A frontmatter field (`ferrule guide show board`)
  state                      open, inprogress, inreview or done
  log:<key>                  The value last logged under <key>: a `<key>=`
                             event's or a `<key>=` on any line, so
                             `log:commit` is the last `commit=<sha>`; a
                             line's free text never counts
  pass-sha                   The `sha=` of the last `gauntlet=pass` line

Examples:
  ferrule board field <slug> lane
  ferrule board field <slug>.s1 log:commit
  ferrule board field <slug>.s1 pass-sha
Exit codes: 0 ok, 4 internal error.
See: ferrule guide show board";

const NEW_AFTER_LONG_HELP: &str = "\
Output:
  (nothing)                      The card was created; exit 0.
Refusals:
  card-new=refused reason=exists The card exists already; pick another id, or
                                 change it with `ferrule board set`
  card-new=reject reason=key key=<k>
                                 <k> is not a field of this kind of card; the
                                 hint lists the allowed ones (see Fields)
  card-new=reject reason=arg arg=<a>
                                 A field argument without `=`; write key=value
  card-new=reject reason=owned key=<k>
                                 Only a ferrule command sets <k> (gate2,
                                 gate3, gate4, branch, worktree, issue, pr);
                                 the hint names it
  card-new=reject reason=state state=done
                                 A stage card cannot start done; create it
                                 open and move it once its commit is verified
Every refusal ends in `hint=<fix>`. Nothing is created on a refusal.
Fields (`ferrule board set` takes the same):
  stage card <slug>.s<n>         title, blocked-by, files, branch, worktree,
                                 stop. `ferrule gate approve <slug> gate2`
                                 creates the stage cards and sets title,
                                 blocked-by, files, branch and stop itself.
  parent card <slug>             kind, epic, slices, issue, pr, branch, lane,
                                 gate1, with-arch, with-plan, skipped, arch,
                                 no-arch, gate2, gate3, gate4, split,
                                 upstream, artifacts, worktree. The workflow
                                 sets these itself, so do not set them by
                                 hand: gate1, arch, gate2, gate3, gate4 and
                                 lane (`ferrule gate approve`), branch
                                 (approve gate2), kind and epic (`run init`
                                 of a slice, `gate approve gate1 --epic`),
                                 issue, slices and split (`ferrule issue
                                 file`; gate2 files the issue too), pr
                                 (`ferrule pr open`, or the first `pr push`).
Exit codes: 0 ok, 1 refused (exists), 2 rejected input, 4 internal error.
See: ferrule guide show board";

const SET_AFTER_LONG_HELP: &str = "\
Output:
  (nothing)                      The fields were written, all in one
                                 transaction; exit 0.
Refusals:
  card-set=refused reason=card   No such card; check it with
                                 `ferrule board list`
  card-set=reject reason=key key=<k>
                                 <k> is not a field of this kind of card; the
                                 hint lists the allowed ones, also under
                                 Fields in `ferrule board new --help` (do not
                                 hand-set the ones the workflow commands
                                 write: gate1, arch, gate2, gate3, gate4,
                                 lane, branch, issue, pr, slices, split);
                                 every key is checked first, so nothing is
                                 written
  card-set=reject reason=arg arg=<a>
                                 A field argument without `=`; write key=value;
                                 nothing is written
  card-set=reject reason=owned key=<k>
                                 Only a ferrule command gives <k> a value
                                 (gate2, gate3, gate4, branch, worktree,
                                 issue, pr); the hint names it. `<k>=`
                                 clears one; nothing is written
Every refusal ends in `hint=<fix>`.
Exit codes: 0 ok, 1 refused (card), 2 rejected input, 4 internal error.
See: ferrule guide show board";

const MOVE_AFTER_LONG_HELP: &str = "\
Output:
  (nothing)                      Moved, and `moved=<state>` logged on the card.
                                 Moving to the card's own state is a no-op.
  pr-body=pushed                 The run has a PR; its draft body was
                                 re-rendered and pushed.
  pr-body=unchanged              The run has a PR; the body did not change.
  pr-body=refused reason=stage-ref stage-ref=<t> hint=<fix>
                                 The body names a stage <t> that does not exist.
  pr-body=failed                 The host could not be reached; the move
                                 stands (details on stderr).
Refusals:
  card-move=refused reason=card  No such card; check it with
                                 `ferrule board list`
  card-move=refused reason=<r> from=<s> to=<s> hint=<fix>
                                 The board forbids the move; nothing changes.
  card-log=reject reason=length event=moved hint=<fix>
                                 The `moved=` line would be over the log line
                                 length; nothing changes (exit 2).
Reasons:
  transition                     Not an allowed move for this kind of card; the
                                 hint names the moves allowed from its state:
                                   stage:  open -> inprogress
                                           inprogress -> inreview
                                           inreview -> done | inprogress
                                           done -> inprogress
                                   parent: open -> inprogress (gate2 set), or
                                             inreview (epic only)
                                           inprogress <-> inreview
                                           done -> nowhere
  blocked-by                     A stage moved to inprogress while a card in its
                                 `blocked-by` is not done
  commit                         A stage moved to done with no `commit=` in its
                                 log; commit it with `ferrule stage commit`
  unverified                     A stage moved to done whose `commit=` sha no
                                 run `gauntlet=pass` covers; `ferrule ci gauntlet`
                                 then `ferrule pr push` logs one
  gate2                          The parent moved to inprogress before its
                                 `gate2` field is set; `ferrule gate approve`
Exit codes: 0 ok, 1 refused (no such card), 2 refused (reason) or rejected
line, 4 internal error.
See: ferrule guide show board";

const LOG_AFTER_LONG_HELP: &str = "\
Output:
  (nothing)                      The line was appended, stamped with the UTC
                                 minute; exit 0.
Refusals:
  card-log=refused reason=card   No such card; check it with
                                 `ferrule board list`
  card-log=reject reason=<r> event=<e> hint=<fix>
                                 The vocabulary refuses the line; nothing is
                                 appended. <e> is the event without its `=`.
Reasons:
  event                          Not an event; `ferrule board vocab` lists them
  key                            A `key=value` the event does not take (a
                                 `stop` also needs `kind=`)
  scope                          The event is for the other card kind (stage
                                 or parent)
  value                          A value, positional word or `-- text` the
                                 event does not accept: an empty or quoted
                                 value, a value outside the event's set, text
                                 where none is allowed or none where it is
                                 required, a missing `origin=` on `spec-delta`
  length                         The line, timestamp included, is over 160
                                 characters
  budget                         The stage card has used its `read` budget
                                 (3, or `read_budget` of `ferrule.toml`); never
                                 read past it: stop with status=blocked
                                 (see `ferrule guide show implementer`)
  reason=typed                   A `note` that starts with an event's name;
                                 `use=<event>` names the event to log instead
  reason=inprogress              `commit=` on a card still in `inprogress`;
                                 `move=inreview` says to move it first
  reason=owner                   An event only a ferrule command writes (the
                                 `owner` column of `board vocab` says
                                 `ferrule`), e.g. `gauntlet=`; the hint names
                                 the command that logs it
  The `hint=` of `reason=value` and `reason=key` names what that event
  takes, e.g. the one-of set (`checks=` takes pass|fail), required keys
  (`stop` needs `kind=question|approval|stage|blocked`; `spec-delta` needs
  `origin=user|review|impl`) and whether `-- text` is required, optional or
  not allowed.
  `[ids]` in a hint or in `board vocab` is an optional list of fixed spec IDs
  like MB-R-001, comma-separated without spaces if several (MB-R-001,MB-R-002).
  The hint of a value refusal names what that event takes.
Exit codes: 0 ok, 1 refused (no such card), 2 rejected line, 4 internal error.
See: ferrule guide show board

Examples:
  ferrule board log <slug>.s1 test-red MB-R-001 -- parses_empty: panicked
  ferrule board log <slug>.s1 test-red MB-R-001,MB-R-002 -- two tests: red
  ferrule board log <slug>.s1 read path=src/lib.rs -- what the plan left out
  ferrule board vocab               every event and its arguments";

const VOCAB_AFTER_LONG_HELP: &str = "\
Output:
  A table, one event per line, with the columns
  scope       stage, parent or both: the cards the event may be logged on
  event       The name; a trailing `=` means the value follows it (`commit=`)
  values      For a valued event: `any` or the allowed values, `|`-separated
  positional  Words before the keys: `id,kind` (a spec ID, then added,
              changed or dropped) or `[ids]` (optional fixed spec IDs like MB-R-001, comma-separated
              without spaces if several)
  keys        The `key=value` arguments the event accepts
  text        Whether `-- <free text>` is no, yes or required
  owner       `agent` for an event `board log` takes; `ferrule` for one only a
              ferrule command logs (`board log` refuses it, reason=owner)
Filters: --event <name> keeps that event (with or without its `=`); --kind
stage|parent keeps the events that card kind takes (`both` events count for
either).
Refusals:
  vocab=none hint=no such event for this card kind; ferrule board vocab lists them all
              No event matches the filters; exit 1.
Exit codes: 0 ok, 1 nothing matched, 4 internal error.
See: ferrule guide show board";

const IMPORT_AFTER_LONG_HELP: &str = "\
Output:
  import=ok cards=<n>            All <n> cards were loaded (frontmatter, log).
Refusals:
  import=refused reason=invalid detail=\"<text>\" hint=<fix>
                                 Nothing was loaded. <text> is
                                 `<path>: <problem>` for a card file whose
                                 `id:` or `parent:` disagrees with its name,
                                 whose `state:` is not the state directory it
                                 sits in, that has a key its kind does not
                                 carry, or whose log line stamp is not
                                 `YYYY-MM-DDTHH:MM`, `card <id> is already on
                                 the board`, an I/O error or `state database:
                                 ...`. Fix the file or the board and run it
                                 again.
Exit codes: 0 ok, 2 refused, 4 internal error.
See: ferrule guide show board";

const HINT_KEY_VALUE: &str = "pass each field as key=value";
const HINT_EXISTS: &str = "pick another id, or change the card with `ferrule board set`";

fn hint_field(card: &str) -> String {
    board::fields_hint(board::CardId::parse(card).stage.is_some())
}
const HINT_GIT: &str =
    "git commit failed; stage the changes with `git add` and read the message git prints on stderr";

#[derive(Debug, Subcommand)]
pub enum BoardCommand {
    /// List every card with its state and title.
    #[command(after_long_help = BOARD_AFTER_LONG_HELP)]
    List {
        /// Only cards in this state; repeat for several.
        #[arg(long, value_enum)]
        state: Vec<CardState>,
        /// Only this run's cards: its parent and its stages.
        #[arg(long, value_name = "SLUG")]
        run: Option<String>,
    },
    /// Print the log lines of a run's parent and stage cards.
    #[command(after_long_help = EVENTS_AFTER_LONG_HELP)]
    Events {
        /// The run's slug.
        slug: Slug,
        /// Only the lines of this event, e.g. `follow-up` or `spec-delta`.
        #[arg(long, value_name = "NAME")]
        event: Option<String>,
        /// Only the latest n lines.
        #[arg(long, value_name = "N")]
        last: Option<usize>,
    },
    /// Print a card: frontmatter and log.
    #[command(after_long_help = SHOW_AFTER_LONG_HELP)]
    Show {
        /// Card id: `<slug>` or `<slug>.s<n>`.
        #[arg(value_parser = ferrule_core::card::board::parse_card_id)]
        card: String,
        /// Only the frontmatter fields.
        #[arg(long, conflicts_with_all = ["log", "last", "event"])]
        fields: bool,
        /// Only the log.
        #[arg(long)]
        log: bool,
        /// Only the latest n log lines.
        #[arg(long, value_name = "N")]
        last: Option<usize>,
        /// Only the log lines of this event, e.g. `commit` or `gauntlet`.
        #[arg(long, value_name = "NAME")]
        event: Option<String>,
    },
    /// Print one value of a card; nothing when absent.
    #[command(after_long_help = FIELD_AFTER_LONG_HELP)]
    Field {
        /// Card id: `<slug>` or `<slug>.s<n>`.
        #[arg(value_parser = ferrule_core::card::board::parse_card_id)]
        card: String,
        /// A frontmatter field, `log:<key>` (the last `<key>=` in the log),
        /// `pass-sha` (the sha of the last `gauntlet=pass`) or `state`.
        field: String,
    },
    /// Create a card.
    #[command(after_long_help = NEW_AFTER_LONG_HELP)]
    New {
        /// Card id: `<slug>` or `<slug>.s<n>`.
        #[arg(value_parser = ferrule_core::card::board::parse_card_id)]
        card: String,
        /// The state the card starts in.
        #[arg(long, value_enum, default_value_t = CardState::Open)]
        state: CardState,
        /// Frontmatter fields, `key=value` each.
        #[arg(value_name = "KEY=VALUE")]
        fields: Vec<String>,
    },
    /// Set frontmatter fields; an empty value removes one.
    #[command(after_long_help = SET_AFTER_LONG_HELP)]
    Set {
        /// Card id: `<slug>` or `<slug>.s<n>`.
        #[arg(value_parser = ferrule_core::card::board::parse_card_id)]
        card: String,
        /// Frontmatter fields, `key=value` each; `key=` removes one.
        #[arg(required = true, value_name = "KEY=VALUE")]
        fields: Vec<String>,
    },
    /// Move a card to another state, if the board allows it.
    #[command(after_long_help = MOVE_AFTER_LONG_HELP)]
    Move {
        /// Card id: `<slug>` or `<slug>.s<n>`.
        #[arg(value_parser = ferrule_core::card::board::parse_card_id)]
        card: String,
        /// The state to move to.
        state: CardState,
    },
    /// Append an event to a card's log.
    ///
    /// Refuses an event, key or value the vocabulary does not accept, naming
    /// what it expects.
    #[command(after_long_help = LOG_AFTER_LONG_HELP)]
    Log {
        /// Card id: `<slug>` or `<slug>.s<n>`.
        #[arg(value_parser = ferrule_core::card::board::parse_card_id)]
        card: String,
        /// `<name>` or `<name>=<value>`; `board vocab` lists them.
        event: String,
        /// The event's arguments: `[positional…] [key=value…] [-- free text]`.
        #[arg(
            trailing_var_arg = true,
            allow_hyphen_values = true,
            value_name = "ARG"
        )]
        args: Vec<String>,
    },
    /// List the events `board log` accepts.
    #[command(after_long_help = VOCAB_AFTER_LONG_HELP)]
    Vocab {
        /// Only this event, with or without its `=`.
        #[arg(long, value_name = "NAME")]
        event: Option<String>,
        /// Only the events this kind of card takes.
        #[arg(long, value_enum)]
        kind: Option<CardKind>,
    },
    /// Load a markdown board into the state database: all cards or none.
    #[command(after_long_help = IMPORT_AFTER_LONG_HELP)]
    Import {
        /// Directory holding `open/`, `inprogress/`, `inreview/` and
        /// `done/`, one `<id>.md` per card.
        board: PathBuf,
    },
}

/// The board's database; `None` when there is none yet, which reads as an
/// empty board.
pub(crate) fn store(create: bool) -> io::Result<Option<Board>> {
    Board::open(&project::main_checkout(), create).map_err(internal)
}

/// The board's database when `id` names a card on it; `None` otherwise.
pub(crate) fn card_store(id: &str) -> io::Result<Option<Board>> {
    match store(false)? {
        Some(s) if s.read(|r| r.state(id)).map_err(internal)?.is_some() => Ok(Some(s)),
        _ => Ok(None),
    }
}

fn state(s: CardState) -> State {
    match s {
        CardState::Open => State::Open,
        CardState::Inprogress => State::InProgress,
        CardState::Inreview => State::InReview,
        CardState::Done => State::Done,
    }
}

/// `key=value` arguments; the first without `=` is returned as the error.
fn key_values(args: &[String]) -> Result<Vec<(String, String)>, String> {
    args.iter()
        .map(|a| {
            a.split_once('=')
                .map(|(k, v)| (k.to_owned(), v.to_owned()))
                .ok_or_else(|| a.clone())
        })
        .collect()
}

/// `board log`'s words after the event, `--` included: clap consumes a `--`
/// that comes right after the event as its own escape, so it is taken back
/// from the raw arguments, whose tail the parsed words are.
fn with_separator(args: &[String]) -> Vec<String> {
    // Lossy, as main reads them: `env::args` panics on a word that is not
    // UTF-8.
    let raw: Vec<String> = std::env::args_os()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    let separated = raw
        .len()
        .checked_sub(args.len() + 1)
        .is_some_and(|i| raw[i] == "--");
    let mut out = Vec::with_capacity(args.len() + 1);
    if separated {
        out.push("--".to_owned());
    }
    out.extend(args.iter().cloned());
    out
}

pub fn run(command: &BoardCommand, style: Option<Style>, out: &mut impl Write) -> io::Result<u8> {
    match command {
        BoardCommand::Field { card, field: f } => field(card, f, out),
        BoardCommand::Log { card, event, args } => log(card, event, &with_separator(args), out),
        BoardCommand::Move { card, state: to } => move_card(card, state(*to), out),
        BoardCommand::Show {
            card,
            fields,
            log,
            last,
            event,
        } => {
            let part = ShowPart {
                fields: !*log,
                log: !*fields,
                last: *last,
                event: event.as_deref(),
            };
            show(card, &part, style, out)
        }
        BoardCommand::Events { slug, event, last } => events(slug, event.as_deref(), *last, out),
        BoardCommand::List { state: states, run } => {
            let states: Vec<State> = states.iter().map(|s| state(*s)).collect();
            list(&states, run.as_deref(), style, out)
        }
        BoardCommand::Import { board } => import(board, out),
        BoardCommand::Vocab { event, kind } => {
            let scope = kind.map(|k| match k {
                CardKind::Stage => vocab::Scope::Stage,
                CardKind::Parent => vocab::Scope::Parent,
            });
            if let Some(table) = vocab::table_for(event.as_deref(), scope) {
                write!(out, "{table}")?;
                return Ok(exit::OK);
            }
            writeln!(
                out,
                "vocab=none hint=no such event for this card kind; ferrule board vocab lists them all"
            )?;
            Ok(exit::REFUSED)
        }
        BoardCommand::New {
            card,
            state: st,
            fields,
        } => new(card, state(*st), fields, out),
        BoardCommand::Set { card, fields } => set(card, fields, out),
    }
}

fn field(card: &str, field: &str, out: &mut impl Write) -> io::Result<u8> {
    let Some(s) = store(false)? else {
        return Ok(exit::OK);
    };
    let value = s
        .read(|r| match field {
            "state" => Ok(r.state(card)?.map(|st| st.as_str().to_owned())),
            "pass-sha" => Ok(r.pass_shas(card)?.pop()),
            f => match f.strip_prefix("log:") {
                Some(key) => r.last_logged(card, key),
                None => r.field(card, f),
            },
        })
        .map_err(internal)?;
    if let Some(v) = value {
        writeln!(out, "{v}")?;
    }
    Ok(exit::OK)
}

fn log(card: &str, event: &str, args: &[String], out: &mut impl Write) -> io::Result<u8> {
    let Some(mut s) = store(false)? else {
        return refuse(out, &Refusal::card("card-log"));
    };
    let config = Config::load(&project::main_checkout()).map_err(internal)?;
    match s.write(|w| {
        w.log_with_budget(
            card,
            event,
            args,
            &board::now(),
            config.thresholds.read_budget,
            true,
        )
    }) {
        Ok(()) => Ok(exit::OK),
        Err(LogError::Card) => refuse(out, &Refusal::card("card-log")),
        Err(LogError::Reject(r)) => refuse(out, &r.refusal()),
        Err(LogError::Sql(e)) => Err(internal(e)),
    }
}

fn move_card(card: &str, to: State, out: &mut impl Write) -> io::Result<u8> {
    let Some(mut s) = store(false)? else {
        return refuse(out, &Refusal::card("card-move"));
    };
    match s.write(|w| w.move_card(card, to, &board::now())) {
        Ok(()) => {
            sync_draft(&mut s, card, out)?;
            Ok(exit::OK)
        }
        Err(e) => match e.refusal(card, to) {
            Some(r) => refuse(out, &r),
            None => Err(internal(e)),
        },
    }
}

fn show(card: &str, part: &ShowPart, style: Option<Style>, out: &mut impl Write) -> io::Result<u8> {
    let shown = match store(false)? {
        Some(s) => s
            .read(|r| {
                r.show(card)?
                    .map(|text| Ok::<_, StoreError>((text, r.log_lines(card, part.event)?)))
                    .transpose()
            })
            .map_err(internal)?,
        None => None,
    };
    if let Some((text, log)) = shown {
        let text = part.cut(&text, log);
        match style {
            Some(s) if part.fields => write!(out, "{}", styled_card(&text, s))?,
            Some(s) => write!(out, "{}", styled_card(&format!("---\n---\n{text}"), s))?,
            None => write!(out, "{text}")?,
        }
        Ok(exit::OK)
    } else {
        refuse(out, &Refusal::card("card-show"))
    }
}

/// `board::show`'s card as aligned fields over its log, timestamps dimmed.
fn styled_card(text: &str, s: Style) -> String {
    let mut parts = text.splitn(3, "---\n");
    let fields = parts.nth(1).unwrap_or("");
    let log = parts.next().unwrap_or("");
    let pairs: Vec<(&str, &str)> = fields.lines().filter_map(|l| l.split_once(": ")).collect();
    let w = pairs.iter().map(|(k, _)| k.len()).max().unwrap_or(0);
    let mut out = String::new();
    for (k, v) in pairs {
        let v = if k == "state" {
            term::state(v, s)
        } else {
            term::sanitize(v)
        };
        let _ = writeln!(out, "{}  {v}", term::strong(&format!("{k:<w$}"), s));
    }
    if !log.is_empty() {
        out.push('\n');
    }
    for line in log.lines() {
        let (at, body) = line.split_once(' ').unwrap_or((line, ""));
        let _ = writeln!(out, "{}  {}", term::faint(at, s), term::sanitize(body));
    }
    out
}

fn events(
    slug: &str,
    event: Option<&str>,
    last: Option<usize>,
    out: &mut impl Write,
) -> io::Result<u8> {
    let Some(s) = card_store(slug)? else {
        return refuse(out, &Refusal::card("events"));
    };
    let mut lines: Vec<String> = s
        .read(|r| r.run_log_lines(slug, event))
        .map_err(internal)?
        .into_iter()
        .map(|(card, at, body)| format!("{card} {at} {body}"))
        .collect();
    if let Some(n) = last {
        lines.drain(..lines.len().saturating_sub(n));
    }
    for line in lines {
        writeln!(out, "{line}")?;
    }
    Ok(exit::OK)
}

fn list(
    states: &[State],
    run: Option<&str>,
    style: Option<Style>,
    out: &mut impl Write,
) -> io::Result<u8> {
    let Some(s) = store(false)? else {
        return Ok(exit::OK);
    };
    let cards = s
        .read(|r| {
            r.all()?
                .into_iter()
                .filter(|(id, st)| {
                    (states.is_empty() || states.contains(st))
                        && run.is_none_or(|run| board::CardId::parse(id).run == run)
                })
                .map(|(id, st)| {
                    let title = r.field(&id, "title")?;
                    Ok::<_, StoreError>((id, st, title))
                })
                .collect::<Result<Vec<_>, _>>()
        })
        .map_err(internal)?;
    for (id, st, title) in cards {
        let state = match style {
            Some(style) => format!(
                "{}{}",
                term::state(st.as_str(), style),
                " ".repeat(10 - st.as_str().len())
            ),
            None => format!("{:<10}", st.as_str()),
        };
        match title {
            Some(t) => writeln!(out, "{state} {id}  {t}")?,
            None => writeln!(out, "{state} {id}")?,
        }
    }
    Ok(exit::OK)
}

/// `<verb>=reject reason=arg`: a field argument without `=`.
fn bad_arg(verb: &'static str, arg: &str) -> Refusal {
    Refusal::new(verb, "arg")
        .reject()
        .field("arg", arg)
        .hint(HINT_KEY_VALUE)
        .exit(Exit::Usage)
}

/// `<verb>=reject reason=key`: not a field of this kind of card.
fn bad_key(verb: &'static str, key: &str, card: &str) -> Refusal {
    Refusal::new(verb, "key")
        .reject()
        .field("key", key)
        .hint(hint_field(card))
        .exit(Exit::Usage)
}

/// A value for a key only ferrule commands set, refused naming the
/// command; clearing one is not refused.
fn owned(verb: &'static str, fields: &[(String, String)]) -> Option<Refusal> {
    fields.iter().find_map(|(k, v)| {
        let by = board::field_owner(k).filter(|_| !v.is_empty())?;
        Some(
            Refusal::new(verb, "owned")
                .reject()
                .field("key", k)
                .hint(format!("only ferrule sets {k}, with `{by}`"))
                .exit(Exit::Usage),
        )
    })
}

fn new(card: &str, st: State, fields: &[String], out: &mut impl Write) -> io::Result<u8> {
    let fields = match key_values(fields) {
        Ok(f) => f,
        Err(arg) => return refuse(out, &bad_arg("card-new", &arg)),
    };
    if let Some(r) = owned("card-new", &fields) {
        return refuse(out, &r);
    }
    if st == State::Done && board::CardId::parse(card).stage.is_some() {
        let r = Refusal::new("card-new", "state")
            .reject()
            .field("state", st)
            .hint("create the stage open; `ferrule board move` takes it to done once its commit is verified")
            .exit(Exit::Usage);
        return refuse(out, &r);
    }
    let Some(mut s) = store(true)? else {
        unreachable!("store(true) creates the database");
    };
    match s.write(|w| w.create(card, st, &fields, &board::now())) {
        Ok(()) => Ok(exit::OK),
        Err(CardError::Exists) => {
            refuse(out, &Refusal::new("card-new", "exists").hint(HINT_EXISTS))
        }
        Err(CardError::Field(k)) => refuse(out, &bad_key("card-new", &k, card)),
        Err(CardError::Card) => unreachable!("create never looks a card up"),
        Err(CardError::Id) => unreachable!("the card id was checked when parsed"),
        Err(CardError::Sql(e)) => Err(internal(e)),
    }
}

fn set(card: &str, fields: &[String], out: &mut impl Write) -> io::Result<u8> {
    let fields = match key_values(fields) {
        Ok(f) => f,
        Err(arg) => return refuse(out, &bad_arg("card-set", &arg)),
    };
    if let Some(r) = owned("card-set", &fields) {
        return refuse(out, &r);
    }
    let Some(mut s) = store(false)? else {
        return refuse(out, &Refusal::card("card-set"));
    };
    match s.write(|w| w.set_all(card, &fields)) {
        Ok(()) => Ok(exit::OK),
        Err(CardError::Card) => refuse(out, &Refusal::card("card-set")),
        Err(CardError::Field(k)) => refuse(out, &bad_key("card-set", &k, card)),
        Err(CardError::Exists | CardError::Id) => unreachable!("set never creates a card"),
        Err(CardError::Sql(e)) => Err(internal(e)),
    }
}

fn import(board_dir: &Path, out: &mut impl Write) -> io::Result<u8> {
    let Some(mut s) = store(true)? else {
        unreachable!("store(true) creates the database");
    };
    match s.import(board_dir, &board::now()) {
        Ok(n) => {
            writeln!(out, "import=ok cards={n}")?;
            Ok(exit::OK)
        }
        Err(e) => {
            let r = Refusal::new("import", "invalid")
                .detail(e.to_string())
                .hint("fix the file or the board the detail names, then import again")
                .exit(Exit::Usage);
            refuse(out, &r)
        }
    }
}

/// `f` on the publish run for `slug`, over the open board `s`.
pub(crate) fn run_on<T>(
    s: &mut Board,
    config: &Config,
    slug: &str,
    f: impl FnOnce(&mut publish::Run) -> T,
) -> T {
    let layout = ferrule_core::project::Layout::new(project::main_checkout());
    let now = board::now();
    let mut run = publish::Run::new(s, &layout, config, &layout.root, slug, &now);
    f(&mut run)
}

/// After a move, the run's draft PR body re-rendered and pushed when the
/// run has a PR. A failure to reach the host does not undo the move.
#[allow(clippy::redundant_closure_for_method_calls)]
pub(crate) fn sync_draft(s: &mut Board, card: &str, out: &mut impl Write) -> io::Result<()> {
    let config = Config::load(&project::main_checkout()).map_err(internal)?;
    match run_on(s, &config, publish::run_of(card), |r| r.sync_draft()) {
        Ok(Some(sync)) => writeln!(out, "{sync}"),
        Ok(None) => Ok(()),
        Err(e) => {
            eprintln!("pr body: {e}");
            writeln!(out, "pr-body=failed")
        }
    }
}

/// The checks before a commit, the commit, then `commit=` on the card.
pub(super) fn commit(
    card: Option<&str>,
    message: &[String],
    out: &mut impl Write,
) -> io::Result<u8> {
    let here = std::env::current_dir()?;
    let config = ferrule_core::config::Config::load(&project::main_checkout()).map_err(internal)?;
    let branch = ferrule_core::git::out(&here, &["symbolic-ref", "--short", "-q", "HEAD"]);
    if branch.as_deref() == Some(config.main_branch.as_str()) {
        let hint = format!(
            "commit on the run's branch, not {}: `ferrule gate approve <slug> gate2` creates it and its worktree; with --no-run, check out another branch first",
            config.main_branch
        );
        let r = Refusal::new("commit", "main")
            .field("branch", &config.main_branch)
            .hint(hint);
        return refuse(out, &r);
    }
    let mut store = None;
    if let Some(id) = card {
        let Some(s) = super::board::store(false)? else {
            return refuse(out, &Refusal::card("commit"));
        };
        let run = board::CardId::parse(id).run;
        let (state, want) = s
            .read(|r| Ok::<_, StoreError>((r.state(id)?, r.field(run, "branch")?)))
            .map_err(internal)?;
        match state {
            None => return refuse(out, &Refusal::card("commit")),
            Some(State::InReview) => {}
            Some(other) => {
                let r = Refusal::new("commit", "state")
                    .field("state", other)
                    .hint(format!(
                        "move the card to inreview first: `ferrule board move {id} inreview`"
                    ));
                return refuse(out, &r);
            }
        }
        let want = want.filter(|b| !b.is_empty());
        if let Some(want) = want
            && branch.as_deref() != Some(want.as_str())
        {
            let r = Refusal::new("commit", "branch")
                .field("want", &want)
                .hint(format!(
                    "check out {want}, the run's branch, and commit there"
                ));
            return refuse(out, &r);
        }
        store = Some(s);
    }
    let text = message.join("\n\n");
    let text = ferrule_core::hook::attribution::strip(&text).unwrap_or(text);
    let mut child = std::process::Command::new("git")
        .arg("-C")
        .arg(&here)
        .args(["commit", "-q", "-F", "-"])
        .stdin(std::process::Stdio::piped())
        .spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        // git quitting before it read the message closes the pipe: its exit
        // status, not the broken pipe, is the answer.
        match stdin.write_all(format!("{text}\n").as_bytes()) {
            Err(e) if e.kind() != io::ErrorKind::BrokenPipe => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(e);
            }
            _ => {}
        }
    }
    if !child.wait()?.success() {
        return refuse(out, &Refusal::new("commit", "git").hint(HINT_GIT));
    }
    let sha = ferrule_core::git::out(&here, &["rev-parse", "--short", "HEAD"]).unwrap_or_default();
    write!(out, "commit=done sha={sha}")?;
    if let (Some(mut s), Some(id)) = (store, card) {
        if let Err(e) = s.write(|w| w.log(id, &format!("commit={sha}"), &[], &board::now())) {
            writeln!(out, " card-log=failed")?;
            eprintln!("stage commit: committed {sha} but could not log it on {id}: {e}");
            return Ok(exit::REFUSED);
        }
        write!(out, " card={id}")?;
    }
    writeln!(out)?;
    Ok(exit::OK)
}
