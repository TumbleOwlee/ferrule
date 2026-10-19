use std::fmt::Write as _;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use clap::{Subcommand, ValueEnum};
use ferrule_core::card::board::{self, CardError, LogError, MoveError, State};
use ferrule_core::card::vocab;
use ferrule_core::config::Config;
use ferrule_core::pr::publish;
use ferrule_core::project;
use ferrule_core::store::{OpenError, Store};
use ferrule_core::term::{self, Style};

use super::exit;

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

const BOARD_AFTER_LONG_HELP: &str = "\
Output:
  <state> <id>  <title>      One line per card, parents before their stages;
                             the title is the card's `title` field, if any.
                             Nothing when there is no board yet.
Exit codes: 0 always.
See: ferrule guide board";

const SHOW_AFTER_LONG_HELP: &str = "\
Output:
  ---                        Markdown: frontmatter between `---` fences (id,
  id: <id>                   parent for a stage card, state, then the fields),
  state: <state>             then one `<UTC minute> <event> ...` line per log
  ---                        entry, oldest first.
Refusals:
  card-show=card             No such card; check the id with
                             `ferrule card board` (hint= says so)
Exit codes: 0 ok, 1 refused.
See: ferrule guide board";

const FIELD_AFTER_LONG_HELP: &str = "\
Output:
  <value>                    The value on one line. Nothing, with exit 0, when
                             the card, the field or the log key is absent.
Fields:
  <name>                     A frontmatter field (`ferrule guide board`)
  state                      open, inprogress, inreview or done
  log:<key>                  The value of the last `<key>=` token in the log;
                             the event word counts, so `log:commit` is the
                             last `commit=<sha>`
  pass-sha                   The `sha=` of the last `gauntlet=pass` line

Examples:
  ferrule card field <slug> lane
  ferrule card field <slug>.s1 log:commit
  ferrule card field <slug>.s1 pass-sha
Exit codes: 0 always.
See: ferrule guide board";

const NEW_AFTER_LONG_HELP: &str = "\
Output:
  (nothing)                      The card was created; exit 0.
Refusals:
  card-new=exists                The card exists already; pick another id, or
                                 change it with `ferrule card set`
  card-new=reject key=<k>        <k> is not a field of this kind of card (a
                                 stage card `<slug>.s<n>` takes title,
                                 blocked-by, files, branch, worktree, stop; a
                                 parent card the run's fields, see the guide)
  card-new=reject arg=<a>        A field argument without `=`; write key=value
Every refusal ends in `hint=<fix>`. Nothing is created on a refusal.
Exit codes: 0 ok, 1 refused (exists), 2 rejected input.
See: ferrule guide board";

const SET_AFTER_LONG_HELP: &str = "\
Output:
  (nothing)                      The fields were written; exit 0.
Refusals:
  card-set=card                  No such card; check the id with
                                 `ferrule card board`
  card-set=reject key=<k>        <k> is not a field of this kind of card (see
                                 `ferrule card new --help`); fields before it
                                 in the call are already written
  card-set=reject arg=<a>        A field argument without `=`; write key=value;
                                 nothing is written
Every refusal ends in `hint=<fix>`.
Exit codes: 0 ok, 1 refused (card), 2 rejected input.
See: ferrule guide board";

const MOVE_AFTER_LONG_HELP: &str = "\
Output:
  (nothing)                      Moved, and `moved=<state>` logged on the card.
                                 Moving to the card's own state is a no-op.
  pr-body=pushed                 The run has a PR; its draft body was
                                 re-rendered and pushed.
  pr-body=unchanged              The run has a PR; the body did not change.
  pr-body=refused stage-ref=<t>  The body names a stage <t> that does not exist.
  pr-body=failed                 The host could not be reached; the move
                                 stands (details on stderr).
Refusals:
  card-move=card                 No such card; check the id with
                                 `ferrule card board`
  card-move=refused from=<s> to=<s> reason=<r> hint=<fix>
                                 The board forbids the move; nothing changes.
Reasons:
  transition                     Not an allowed move for this kind of card; see
                                 the table in `ferrule guide board`
  blocked-by                     A stage moved to inprogress while a card in its
                                 `blocked-by` is not done
  commit                         A stage moved to done with no `commit=` in its
                                 log; commit it with `ferrule card commit`
  unverified                     A stage moved to done whose `commit=` sha no
                                 run `gauntlet=pass` covers; run the gauntlet
  gate2                          The parent moved to inprogress before its
                                 `gate2` field is set; `ferrule run approve`
Exit codes: 0 ok, 1 refused (no such card), 2 refused (reason).
See: ferrule guide board";

const LOG_AFTER_LONG_HELP: &str = "\
Output:
  (nothing)                      The line was appended, stamped with the UTC
                                 minute; exit 0.
Refusals:
  card-log=card                  No such card; check the id with
                                 `ferrule card board`
  card-log=reject event=<e> reason=<r> hint=<fix>
                                 The vocabulary refuses the line; nothing is
                                 appended. <e> is the event without its `=`.
Reasons:
  event                          Not an event; `ferrule card vocab` lists them
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
                                 (3, or `read_budget` of `ferrule.toml`)
  reason=typed                   A `note` that starts with an event's name;
                                 `use=<event>` names the event to log instead
  reason=inprogress              `commit=` on a card still in `inprogress`;
                                 `move=inreview` says to move it first
Exit codes: 0 ok, 1 refused (no such card), 2 rejected line.
See: ferrule guide board

Examples:
  ferrule card log <slug>.s1 test-red R2 -- parses_empty: panicked
  ferrule card log <slug>.s1 read path=src/lib.rs -- what the plan left out
  ferrule card vocab               every event and its arguments";

const VOCAB_AFTER_LONG_HELP: &str = "\
Output:
  A table, one event per line, with the columns
  scope       stage, parent or both: the cards the event may be logged on
  event       The name; a trailing `=` means the value follows it (`commit=`)
  values      For a valued event: `any` or the allowed values, `|`-separated
  positional  Words before the keys: `id,kind` (a spec ID, then added,
              changed or dropped) or `[ids]` (optional comma-separated IDs)
  keys        The `key=value` arguments the event accepts
  text        Whether `-- <free text>` is no, yes or required
Exit codes: 0 always.
See: ferrule guide board";

const COMMIT_AFTER_LONG_HELP: &str = "\
Output:
  commit=done sha=<sha>          Committed; <sha> is the short hash.
  commit=done sha=<sha> card=<id>
                                 Also logged `commit=<sha>` on the card.
  commit=done sha=<sha> card-log=failed
                                 Committed, but the log write failed (details
                                 on stderr): log it with
                                 `ferrule card log <id> commit=<sha>`; exit 1.
Refusals (checked in this order; each line ends in `hint=<fix>`):
  commit=refused reason=main branch=<b>
                                 On the main branch <b>; commit on the run's
                                 branch
  commit=refused reason=card     No such card; check the id with
                                 `ferrule card board`
  commit=refused reason=state state=<s>
                                 The card is in <s>, not inreview; move it with
                                 `ferrule card move <card> inreview`
  commit=refused reason=branch want=<b>
                                 The run's `branch` field is <b> and another
                                 branch is checked out; check out <b>
  commit=refused reason=git      `git commit` failed (nothing staged, a hook
                                 refused); git's message is on stderr
With --no-run only the main-branch check applies and no card is touched.
Attribution trailers are stripped from the message first.
Exit codes: 0 ok, 1 refused.
See: ferrule guide implementer";

const IMPORT_AFTER_LONG_HELP: &str = "\
Output:
  import=ok cards=<n>            All <n> cards were loaded (frontmatter, log).
Refusals:
  import=refused reason=<text>   Nothing was loaded. <text> is free text to the
                                 end of the line: `<path>: <problem>` for a
                                 card file whose `id:` or `parent:` disagrees
                                 with its name or that has a key its kind does
                                 not carry, `card <id> is already on the
                                 board`, an I/O error or `state database: ...`.
                                 Fix the file or the board and run it again.
Exit codes: 0 ok, 2 refused.
See: ferrule guide board";

const HINT_CARD: &str = "check the id with `ferrule card board`";
const HINT_KEY_VALUE: &str = "pass each field as key=value";
const HINT_EXISTS: &str = "pick another id, or change the card with `ferrule card set`";
const HINT_FIELD: &str = "that key is not a field of this kind of card; see `ferrule guide board`";
const HINT_GIT: &str =
    "git commit failed; stage the changes with `git add` and read the message git prints on stderr";

#[derive(Debug, Subcommand)]
pub enum CardCommand {
    /// List every card with its state and title.
    #[command(after_long_help = BOARD_AFTER_LONG_HELP)]
    Board,
    /// Print a card: frontmatter and log.
    #[command(after_long_help = SHOW_AFTER_LONG_HELP)]
    Show {
        /// Card id: `<slug>` or `<slug>.s<n>`.
        card: String,
    },
    /// Print one value of a card; nothing when absent.
    #[command(after_long_help = FIELD_AFTER_LONG_HELP)]
    Field {
        /// Card id: `<slug>` or `<slug>.s<n>`.
        card: String,
        /// A frontmatter field, `log:<key>` (the last `<key>=` in the log),
        /// `pass-sha` (the sha of the last `gauntlet=pass`) or `state`.
        field: String,
    },
    /// Create a card.
    #[command(after_long_help = NEW_AFTER_LONG_HELP)]
    New {
        /// Card id: `<slug>` or `<slug>.s<n>`.
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
        card: String,
        /// Frontmatter fields, `key=value` each; `key=` removes one.
        #[arg(required = true, value_name = "KEY=VALUE")]
        fields: Vec<String>,
    },
    /// Move a card to another state, if the board allows it.
    #[command(after_long_help = MOVE_AFTER_LONG_HELP)]
    Move {
        /// Card id: `<slug>` or `<slug>.s<n>`.
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
        card: String,
        /// `<name>` or `<name>=<value>`; `card vocab` lists them.
        event: String,
        /// The event's arguments: `[positional…] [key=value…] [-- free text]`.
        #[arg(
            trailing_var_arg = true,
            allow_hyphen_values = true,
            value_name = "ARG"
        )]
        args: Vec<String>,
    },
    /// List the events `card log` accepts.
    #[command(after_long_help = VOCAB_AFTER_LONG_HELP)]
    Vocab,
    /// Commit the staged changes of a stage in review.
    ///
    /// Refuses the main branch and a branch other than the run's, strips
    /// attribution trailers from the message and logs `commit=` on the card.
    #[command(after_long_help = COMMIT_AFTER_LONG_HELP)]
    Commit {
        /// The stage card, `<slug>.s<n>`; omitted with `--no-run`.
        #[arg(required_unless_present = "no_run", conflicts_with = "no_run")]
        card: Option<String>,
        /// The commit message; repeat for more paragraphs.
        #[arg(short, long = "message", required = true)]
        message: Vec<String>,
        /// Commit outside a run: only the branch and trailer rules apply.
        #[arg(long)]
        no_run: bool,
    },
    /// Load a markdown board into the state database: all cards or none.
    #[command(after_long_help = IMPORT_AFTER_LONG_HELP)]
    Import {
        /// Directory holding `open/`, `inprogress/`, `inreview/` and
        /// `done/`, one `<id>.md` per card.
        board: PathBuf,
    },
}

pub(crate) fn sql(e: impl std::fmt::Display) -> io::Error {
    io::Error::other(e.to_string())
}

/// The board's database; `None` when there is none yet, which reads as an
/// empty board.
pub(crate) fn store(create: bool) -> io::Result<Option<Store>> {
    match Store::open(&project::main_checkout(), create) {
        Ok(s) => Ok(Some(s)),
        Err(OpenError::Missing(_)) => Ok(None),
        Err(e) => Err(sql(e)),
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

/// `card log`'s words after the event, `--` included: clap consumes a `--`
/// that comes right after the event as its own escape, so it is taken back
/// from the raw arguments, whose tail the parsed words are.
fn with_separator(args: &[String]) -> Vec<String> {
    let raw: Vec<String> = std::env::args().collect();
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

pub fn run(command: &CardCommand, style: Option<Style>, out: &mut impl Write) -> io::Result<u8> {
    match command {
        CardCommand::Field { card, field: f } => field(card, f, out),
        CardCommand::Log { card, event, args } => log(card, event, &with_separator(args), out),
        CardCommand::Move { card, state: to } => move_card(card, state(*to), out),
        CardCommand::Commit { card, message, .. } => commit(card.as_deref(), message, out),
        CardCommand::Show { card } => show(card, style, out),
        CardCommand::Board => list(style, out),
        CardCommand::Import { board } => import(board, out),
        CardCommand::Vocab => {
            write!(out, "{}", vocab::table())?;
            Ok(exit::OK)
        }
        CardCommand::New {
            card,
            state: st,
            fields,
        } => new(card, state(*st), fields, out),
        CardCommand::Set { card, fields } => set(card, fields, out),
    }
}

fn field(card: &str, field: &str, out: &mut impl Write) -> io::Result<u8> {
    let Some(s) = store(false)? else {
        return Ok(exit::OK);
    };
    let value = match field {
        "state" => board::state(&s.conn, card)
            .map_err(sql)?
            .map(|st| st.as_str().to_owned()),
        "pass-sha" => board::pass_shas(&s.conn, card).map_err(sql)?.pop(),
        f => match f.strip_prefix("log:") {
            Some(key) => board::last_token(&s.conn, card, key).map_err(sql)?,
            None => board::field(&s.conn, card, f).map_err(sql)?,
        },
    };
    if let Some(v) = value {
        writeln!(out, "{v}")?;
    }
    Ok(exit::OK)
}

fn log(card: &str, event: &str, args: &[String], out: &mut impl Write) -> io::Result<u8> {
    let Some(mut s) = store(false)? else {
        writeln!(out, "card-log=card hint={HINT_CARD}")?;
        return Ok(exit::REFUSED);
    };
    let config = Config::load(&project::main_checkout()).map_err(sql)?;
    match board::log_with_budget(
        &mut s.conn,
        card,
        event,
        args,
        &board::now(),
        config.thresholds.read_budget,
    ) {
        Ok(()) => Ok(exit::OK),
        Err(LogError::Card) => {
            writeln!(out, "card-log=card hint={HINT_CARD}")?;
            Ok(exit::REFUSED)
        }
        Err(LogError::Reject(r)) => {
            writeln!(out, "{r}")?;
            Ok(exit::USAGE)
        }
        Err(LogError::Sql(e)) => Err(sql(e)),
    }
}

fn move_card(card: &str, to: State, out: &mut impl Write) -> io::Result<u8> {
    let Some(mut s) = store(false)? else {
        writeln!(out, "card-move=card hint={HINT_CARD}")?;
        return Ok(exit::REFUSED);
    };
    match board::move_card(&mut s.conn, card, to, &board::now()) {
        Ok(()) => {
            sync_draft(&mut s, card, out)?;
            Ok(exit::OK)
        }
        Err(MoveError::Card) => {
            writeln!(out, "card-move=card hint={HINT_CARD}")?;
            Ok(exit::REFUSED)
        }
        Err(MoveError::Refused { from, reason }) => {
            writeln!(
                out,
                "card-move=refused from={from} to={to} reason={} hint={}",
                reason.as_str(),
                reason.hint()
            )?;
            Ok(exit::USAGE)
        }
        Err(MoveError::Sql(e)) => Err(sql(e)),
    }
}

fn show(card: &str, style: Option<Style>, out: &mut impl Write) -> io::Result<u8> {
    let shown = match store(false)? {
        Some(s) => board::show(&s.conn, card).map_err(sql)?,
        None => None,
    };
    if let Some(text) = shown {
        match style {
            Some(s) => write!(out, "{}", styled_card(&text, s))?,
            None => write!(out, "{text}")?,
        }
        Ok(exit::OK)
    } else {
        writeln!(out, "card-show=card hint={HINT_CARD}")?;
        Ok(exit::REFUSED)
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
            v.to_owned()
        };
        let _ = writeln!(out, "{}  {v}", term::strong(&format!("{k:<w$}"), s));
    }
    if !log.is_empty() {
        out.push('\n');
    }
    for line in log.lines() {
        let (at, body) = line.split_once(' ').unwrap_or((line, ""));
        let _ = writeln!(out, "{}  {body}", term::faint(at, s));
    }
    out
}

fn list(style: Option<Style>, out: &mut impl Write) -> io::Result<u8> {
    let Some(s) = store(false)? else {
        return Ok(exit::OK);
    };
    for (id, st) in board::all(&s.conn).map_err(sql)? {
        let state = match style {
            Some(style) => format!(
                "{}{}",
                term::state(st.as_str(), style),
                " ".repeat(10 - st.as_str().len())
            ),
            None => format!("{:<10}", st.as_str()),
        };
        match board::field(&s.conn, &id, "title").map_err(sql)? {
            Some(t) => writeln!(out, "{state} {id}  {t}")?,
            None => writeln!(out, "{state} {id}")?,
        }
    }
    Ok(exit::OK)
}

fn new(card: &str, st: State, fields: &[String], out: &mut impl Write) -> io::Result<u8> {
    let fields = match key_values(fields) {
        Ok(f) => f,
        Err(arg) => {
            writeln!(out, "card-new=reject arg={arg} hint={HINT_KEY_VALUE}")?;
            return Ok(exit::USAGE);
        }
    };
    let Some(mut s) = store(true)? else {
        unreachable!("store(true) creates the database");
    };
    match board::create(&mut s.conn, card, st, &fields, &board::now()) {
        Ok(()) => Ok(exit::OK),
        Err(CardError::Exists) => {
            writeln!(out, "card-new=exists hint={HINT_EXISTS}")?;
            Ok(exit::REFUSED)
        }
        Err(CardError::Field(k)) => {
            writeln!(out, "card-new=reject key={k} hint={HINT_FIELD}")?;
            Ok(exit::USAGE)
        }
        Err(CardError::Card) => unreachable!("create never looks a card up"),
        Err(CardError::Sql(e)) => Err(sql(e)),
    }
}

fn set(card: &str, fields: &[String], out: &mut impl Write) -> io::Result<u8> {
    let fields = match key_values(fields) {
        Ok(f) => f,
        Err(arg) => {
            writeln!(out, "card-set=reject arg={arg} hint={HINT_KEY_VALUE}")?;
            return Ok(exit::USAGE);
        }
    };
    let Some(mut s) = store(false)? else {
        writeln!(out, "card-set=card hint={HINT_CARD}")?;
        return Ok(exit::REFUSED);
    };
    for (k, v) in &fields {
        match board::set(&mut s.conn, card, k, v) {
            Ok(()) => {}
            Err(CardError::Card) => {
                writeln!(out, "card-set=card hint={HINT_CARD}")?;
                return Ok(exit::REFUSED);
            }
            Err(CardError::Field(k)) => {
                writeln!(out, "card-set=reject key={k} hint={HINT_FIELD}")?;
                return Ok(exit::USAGE);
            }
            Err(CardError::Exists) => unreachable!("set never creates a card"),
            Err(CardError::Sql(e)) => return Err(sql(e)),
        }
    }
    Ok(exit::OK)
}

fn import(board_dir: &Path, out: &mut impl Write) -> io::Result<u8> {
    let Some(mut s) = store(true)? else {
        unreachable!("store(true) creates the database");
    };
    match board::import(&mut s.conn, board_dir, &board::now()) {
        Ok(n) => {
            writeln!(out, "import=ok cards={n}")?;
            Ok(exit::OK)
        }
        Err(e) => {
            writeln!(out, "import=refused reason={e}")?;
            Ok(exit::USAGE)
        }
    }
}

/// After a move, the run's draft PR body re-rendered and pushed when the
/// run has a PR. A failure to reach the host does not undo the move.
pub(crate) fn sync_draft(s: &mut Store, card: &str, out: &mut impl Write) -> io::Result<()> {
    let layout = ferrule_core::project::Layout::new(project::main_checkout());
    let config = Config::load(&layout.root).map_err(sql)?;
    let now = board::now();
    let mut run = publish::Run {
        conn: &mut s.conn,
        layout: &layout,
        config: &config,
        root: &layout.root,
        slug: publish::run_of(card),
        now: &now,
    };
    match run.sync_draft() {
        Ok(Some(sync)) => writeln!(out, "{sync}"),
        Ok(None) => Ok(()),
        Err(e) => {
            eprintln!("pr body: {e:?}");
            writeln!(out, "pr-body=failed")
        }
    }
}

/// The checks before a commit, the commit, then `commit=` on the card.
fn commit(card: Option<&str>, message: &[String], out: &mut impl Write) -> io::Result<u8> {
    let here = std::env::current_dir()?;
    let config = ferrule_core::config::Config::load(&project::main_checkout()).map_err(sql)?;
    let branch = ferrule_core::git::out(&here, &["symbolic-ref", "--short", "-q", "HEAD"]);
    if branch.as_deref() == Some(config.main_branch.as_str()) {
        let hint = format!("commit on the run's branch, not {}", config.main_branch);
        writeln!(
            out,
            "commit=refused reason=main branch={} hint={hint}",
            config.main_branch
        )?;
        return Ok(exit::REFUSED);
    }
    let mut store = None;
    if let Some(id) = card {
        let Some(s) = super::card::store(false)? else {
            writeln!(out, "commit=refused reason=card hint={HINT_CARD}")?;
            return Ok(exit::REFUSED);
        };
        match board::state(&s.conn, id).map_err(sql)? {
            None => {
                writeln!(out, "commit=refused reason=card hint={HINT_CARD}")?;
                return Ok(exit::REFUSED);
            }
            Some(State::InReview) => {}
            Some(other) => {
                let hint =
                    format!("move the card to inreview first: `ferrule card move {id} inreview`");
                writeln!(
                    out,
                    "commit=refused reason=state state={} hint={hint}",
                    other.as_str()
                )?;
                return Ok(exit::REFUSED);
            }
        }
        let run = id.split_once('.').map_or(id, |(r, _)| r);
        let want = board::field(&s.conn, run, "branch")
            .map_err(sql)?
            .filter(|b| !b.is_empty());
        if let Some(want) = want
            && branch.as_deref() != Some(want.as_str())
        {
            let hint = format!("check out {want}, the run's branch, and commit there");
            writeln!(out, "commit=refused reason=branch want={want} hint={hint}")?;
            return Ok(exit::REFUSED);
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
        stdin.write_all(text.as_bytes())?;
        stdin.write_all(b"\n")?;
    }
    if !child.wait()?.success() {
        writeln!(out, "commit=refused reason=git hint={HINT_GIT}")?;
        return Ok(exit::REFUSED);
    }
    let sha = ferrule_core::git::out(&here, &["rev-parse", "--short", "HEAD"]).unwrap_or_default();
    write!(out, "commit=done sha={sha}")?;
    if let (Some(mut s), Some(id)) = (store, card) {
        if let Err(e) = board::log(
            &mut s.conn,
            id,
            &format!("commit={sha}"),
            &[],
            &board::now(),
        ) {
            writeln!(out, " card-log=failed")?;
            eprintln!("card commit: committed {sha} but could not log it on {id}: {e:?}");
            return Ok(exit::REFUSED);
        }
        write!(out, " card={id}")?;
    }
    writeln!(out)?;
    Ok(exit::OK)
}
