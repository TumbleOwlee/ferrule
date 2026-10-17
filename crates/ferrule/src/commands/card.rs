use std::io::{self, Write};

use ferrule_core::board::{self, CardError, LogError, MoveError, State};
use ferrule_core::store::{OpenError, Store};
use ferrule_core::{project, vocab};

use super::exit;
use crate::cli::{CardCommand, CardState, ImportArgs};

fn sql(e: impl std::fmt::Display) -> io::Error {
    io::Error::other(e.to_string())
}

/// The board's database; `None` when there is none yet, which reads as an
/// empty board.
fn store(create: bool) -> io::Result<Option<Store>> {
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

pub fn run(command: &CardCommand, out: &mut impl Write) -> io::Result<u8> {
    match command {
        CardCommand::Field { card, field: f } => field(card, f, out),
        CardCommand::Log { card, event, args } => log(card, event, &with_separator(args), out),
        CardCommand::Move { card, state: to } => move_card(card, state(*to), out),
        CardCommand::Show { card } => show(card, out),
        CardCommand::Board => list(out),
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
    match board::log(&mut s.conn, card, event, args, &board::now()) {
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
        Ok(()) => Ok(exit::OK),
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

fn show(card: &str, out: &mut impl Write) -> io::Result<u8> {
    let shown = match store(false)? {
        Some(s) => board::show(&s.conn, card).map_err(sql)?,
        None => None,
    };
    if let Some(text) = shown {
        write!(out, "{text}")?;
        Ok(exit::OK)
    } else {
        writeln!(out, "card-show=card")?;
        Ok(exit::REFUSED)
    }
}

fn list(out: &mut impl Write) -> io::Result<u8> {
    let Some(s) = store(false)? else {
        return Ok(exit::OK);
    };
    for (id, st) in board::all(&s.conn).map_err(sql)? {
        match board::field(&s.conn, &id, "title").map_err(sql)? {
            Some(t) => writeln!(out, "{:<10} {id}  {t}", st.as_str())?,
            None => writeln!(out, "{:<10} {id}", st.as_str())?,
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

pub fn import(args: &ImportArgs, out: &mut impl Write) -> io::Result<u8> {
    let Some(mut s) = store(true)? else {
        unreachable!("store(true) creates the database");
    };
    match board::import(&mut s.conn, &args.board, &board::now()) {
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
