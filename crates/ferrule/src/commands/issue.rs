use std::io::{self, Write};

use ferrule_core::body;
use ferrule_core::config::Config;
use ferrule_core::project::{self, Layout};
use ferrule_core::remote;
use ferrule_core::request::{self, Part};

use super::card::{sql, store};
use super::exit;
use crate::cli::{IssueCommand, RequestPart};

pub fn run(command: &IssueCommand, out: &mut impl Write) -> io::Result<u8> {
    match command {
        IssueCommand::View { issue } => view(issue, out),
        IssueCommand::Request { issue, part } => request(issue, *part, out),
        IssueCommand::Body { slug, slice } => body(slug, *slice, out),
    }
}

fn config() -> io::Result<Config> {
    Config::load(&project::main_checkout()).map_err(sql)
}

fn view(issue: &str, out: &mut impl Write) -> io::Result<u8> {
    match remote::issue(config()?.backend.tracker, issue, &project::root()) {
        Ok(item) => {
            write!(out, "{}", item.text())?;
            Ok(exit::OK)
        }
        Err(e) => {
            eprintln!("issue view: {e}");
            Ok(exit::REFUSED)
        }
    }
}

fn request(issue: &str, part: RequestPart, out: &mut impl Write) -> io::Result<u8> {
    let item = match remote::issue(config()?.backend.tracker, issue, &project::root()) {
        Ok(item) => item,
        Err(e) => {
            eprintln!("issue request: {e}");
            return Ok(exit::REFUSED);
        }
    };
    if part.comments {
        let text = item.text();
        let after = text
            .split_once(" comment(s) ---\n")
            .map_or("", |(_, rest)| rest);
        write!(out, "{}", after.strip_prefix('\n').unwrap_or(after))?;
        return Ok(exit::OK);
    }
    let which = if part.spec {
        Part::Spec
    } else if part.epic {
        Part::Epic
    } else if part.slice_line {
        Part::SliceLine
    } else {
        Part::Request
    };
    for l in request::part(&item.body, which) {
        writeln!(out, "{l}")?;
    }
    Ok(exit::OK)
}

/// Prints an assembled body, or the refusal it ran into.
pub fn print_body(result: Result<String, body::Error>, out: &mut impl Write) -> io::Result<u8> {
    match result {
        Ok(text) => {
            write!(out, "{text}")?;
            Ok(exit::OK)
        }
        Err(body::Error::StageRef(t)) => {
            writeln!(out, "body=refused stage-ref={t}")?;
            Ok(exit::CONTENT)
        }
        Err(body::Error::Missing(p)) => {
            eprintln!("body: missing {}", p.display());
            Ok(exit::REFUSED)
        }
        Err(body::Error::NoSlice(n)) => {
            eprintln!("body: no slice {n} in slices.md");
            Ok(exit::REFUSED)
        }
        Err(body::Error::Empty) => Ok(exit::REFUSED),
        Err(body::Error::Sql(e)) => Err(sql(e)),
    }
}

fn body(slug: &str, slice: Option<u32>, out: &mut impl Write) -> io::Result<u8> {
    let layout = Layout::new(project::main_checkout());
    let s = match store(false)? {
        Some(s) => s,
        None => ferrule_core::store::Store::memory().map_err(sql)?,
    };
    print_body(body::issue(&s.conn, &layout, slug, slice), out)
}
