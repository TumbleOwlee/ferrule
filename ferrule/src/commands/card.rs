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
    Open,
    Inprogress,
    Inreview,
    Done,
}

#[derive(Debug, Subcommand)]
pub enum CardCommand {
    /// List every card with its state and title.
    Board,
    /// Print a card: frontmatter and log.
    Show {
        /// Card id: `<slug>` or `<slug>.s<n>`.
        card: String,
    },
    /// Print one value of a card; nothing when absent.
    Field {
        /// Card id: `<slug>` or `<slug>.s<n>`.
        card: String,
        /// A frontmatter field, `log:<key>` (the last `<key>=` in the log),
        /// `pass-sha` (the sha of the last `gauntlet=pass`) or `state`.
        field: String,
    },
    /// Create a card.
    New {
        /// Card id: `<slug>` or `<slug>.s<n>`.
        card: String,
        #[arg(long, value_enum, default_value_t = CardState::Open)]
        state: CardState,
        /// Frontmatter fields, `key=value` each.
        fields: Vec<String>,
    },
    /// Set frontmatter fields; an empty value removes one.
    Set {
        /// Card id: `<slug>` or `<slug>.s<n>`.
        card: String,
        /// `key=value` each.
        #[arg(required = true)]
        fields: Vec<String>,
    },
    /// Move a card to another state, if the board allows it.
    Move {
        /// Card id: `<slug>` or `<slug>.s<n>`.
        card: String,
        state: CardState,
    },
    /// Append an event to a card's log.
    Log {
        /// Card id: `<slug>` or `<slug>.s<n>`.
        card: String,
        /// `<name>` or `<name>=<value>`; `card vocab` lists them.
        event: String,
        /// `[positional…] [key=value…] [-- free text]`.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// List the events `card log` accepts.
    Vocab,
    /// Commit the staged changes of a stage in review.
    ///
    /// Refuses the main branch and a branch other than the run's, strips
    /// attribution trailers from the message and logs `commit=` on the card.
    Commit {
        /// The stage card, `<slug>.s<n>`; omitted with `--no-run`.
        #[arg(required_unless_present = "no_run", conflicts_with = "no_run")]
        card: Option<String>,
        /// The message; several make paragraphs.
        #[arg(short, long = "message", required = true)]
        message: Vec<String>,
        /// Commit outside a run: only the branch and trailer rules apply.
        #[arg(long)]
        no_run: bool,
    },
    /// Load a markdown board into the state database: all cards or none.
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
        writeln!(out, "card-log=card")?;
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
            writeln!(out, "card-log=card")?;
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
        writeln!(out, "card-move=card")?;
        return Ok(exit::REFUSED);
    };
    match board::move_card(&mut s.conn, card, to, &board::now()) {
        Ok(()) => {
            sync_draft(&mut s, card, out)?;
            Ok(exit::OK)
        }
        Err(MoveError::Card) => {
            writeln!(out, "card-move=card")?;
            Ok(exit::REFUSED)
        }
        Err(MoveError::Refused { from, reason }) => {
            writeln!(
                out,
                "card-move=refused from={from} to={to} reason={}",
                reason.as_str()
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
        writeln!(out, "card-show=card")?;
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
            writeln!(out, "card-new=reject arg={arg}")?;
            return Ok(exit::USAGE);
        }
    };
    let Some(mut s) = store(true)? else {
        unreachable!("store(true) creates the database");
    };
    match board::create(&mut s.conn, card, st, &fields, &board::now()) {
        Ok(()) => Ok(exit::OK),
        Err(CardError::Exists) => {
            writeln!(out, "card-new=exists")?;
            Ok(exit::REFUSED)
        }
        Err(CardError::Field(k)) => {
            writeln!(out, "card-new=reject key={k}")?;
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
            writeln!(out, "card-set=reject arg={arg}")?;
            return Ok(exit::USAGE);
        }
    };
    let Some(mut s) = store(false)? else {
        writeln!(out, "card-set=card")?;
        return Ok(exit::REFUSED);
    };
    for (k, v) in &fields {
        match board::set(&mut s.conn, card, k, v) {
            Ok(()) => {}
            Err(CardError::Card) => {
                writeln!(out, "card-set=card")?;
                return Ok(exit::REFUSED);
            }
            Err(CardError::Field(k)) => {
                writeln!(out, "card-set=reject key={k}")?;
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
        writeln!(
            out,
            "commit=refused reason=main branch={}",
            config.main_branch
        )?;
        return Ok(exit::REFUSED);
    }
    let mut store = None;
    if let Some(id) = card {
        let Some(s) = super::card::store(false)? else {
            writeln!(out, "commit=refused reason=card")?;
            return Ok(exit::REFUSED);
        };
        match board::state(&s.conn, id).map_err(sql)? {
            None => {
                writeln!(out, "commit=refused reason=card")?;
                return Ok(exit::REFUSED);
            }
            Some(State::InReview) => {}
            Some(other) => {
                writeln!(out, "commit=refused reason=state state={}", other.as_str())?;
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
            writeln!(out, "commit=refused reason=branch want={want}")?;
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
        writeln!(out, "commit=refused reason=git")?;
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
