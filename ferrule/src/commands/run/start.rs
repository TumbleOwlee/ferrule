//! Starting a run and putting one back: `run init`, `run reconcile` and
//! `run rebuild`.

use std::io::{self, Write};

use ferrule_core::card::board;
use ferrule_core::config::Config;
use ferrule_core::remote::{self, CodeHost, IssueTracker, Item, ItemRef};
use ferrule_core::run::{self as core_run, Error, gate, rebuild};

use crate::commands::board::{create_store, run_on, store};
use crate::commands::{exit, internal, layout, refuse};

pub(super) fn init(slug: &str, issue: Option<&ItemRef>, out: &mut impl Write) -> io::Result<u8> {
    let layout = layout();
    let config = Config::load(&layout.root).map_err(internal)?;
    let mut s = create_store()?;
    match gate::init(&mut s, &layout, &config, slug, issue, &board::now()) {
        Ok(epic) => {
            write!(out, "init=done slug={slug}")?;
            if let Some(e) = epic {
                write!(out, " kind=slice epic={e}")?;
            }
            writeln!(out)?;
            Ok(exit::OK)
        }
        Err(e) => crate::commands::gate::failed(e, "init", slug, out),
    }
}

pub(super) fn reconcile(slug: &str, local: bool, out: &mut impl Write) -> io::Result<u8> {
    let layout = layout();
    let mut store = store()?;
    let result = match &store {
        Some(s) => s.read(|r| core_run::reconcile(r, &layout, slug)),
        None => Err(Error::Card),
    };
    let mut r = match result {
        Ok(r) => r,
        Err(Error::Card) => {
            eprintln!("reconcile: no card for {slug}");
            return Ok(exit::USAGE);
        }
        Err(Error::Sql(e)) => return Err(internal(e)),
    };
    if !local && let Some(s) = store.as_mut() {
        let config = Config::load(&layout.root).map_err(internal)?;
        match run_on(s, &config, slug, |r| r.reconcile_remote()) {
            Ok(rows) => r.differ.extend(rows),
            Err(e) => r.differ.push(format!("host unreadable: {e}")),
        }
    }
    for d in &r.differ {
        writeln!(out, "differ row={d}")?;
    }
    let next = r.next.as_deref().unwrap_or("none");
    if r.differ.is_empty() {
        writeln!(out, "reconcile=clean next={next}")?;
        Ok(exit::OK)
    } else {
        writeln!(out, "reconcile=differ n={} next={next}", r.differ.len())?;
        Ok(exit::REFUSED)
    }
}

/// The remote host behind `run rebuild`.
struct Remote {
    host: Box<dyn CodeHost>,
    tracker: Box<dyn IssueTracker>,
    pr: ItemRef,
    issue: ItemRef,
}

/// A missing item is `None`; any other failure is the host's error. `gh`
/// reports a missing item only in its message.
fn found<T>(r: Result<T, remote::Error>) -> Result<Option<T>, String> {
    match r {
        Ok(v) => Ok(Some(v)),
        Err(remote::Error::NotFound(_)) => Ok(None),
        Err(remote::Error::Command(m))
            if m.contains("Could not resolve to") || m.to_lowercase().contains("not found") =>
        {
            Ok(None)
        }
        Err(e) => Err(e.to_string()),
    }
}

impl rebuild::Source for Remote {
    fn pr(&self) -> Result<Option<Item>, String> {
        found(self.host.view(&self.pr))
    }

    fn branch(&self) -> Result<Option<String>, String> {
        found(self.host.branch(&self.pr))
    }

    fn issue(&self) -> Result<Option<Item>, String> {
        found(self.tracker.view(&self.issue))
    }
}

pub(super) fn rebuild(issue: &ItemRef, pr: &ItemRef, out: &mut impl Write) -> io::Result<u8> {
    let layout = layout();
    let config = Config::load(&layout.root).map_err(internal)?;
    let main = config.main_branch.clone();
    let source = Remote {
        host: remote::code_host(&config, &layout.root),
        tracker: remote::issue_tracker(&config, &layout.root),
        pr: pr.clone(),
        issue: issue.clone(),
    };
    let mut s = create_store()?;
    let (issue, pr) = (issue.to_string(), pr.to_string());
    match rebuild::rebuild(&mut s, &layout, &main, &issue, &pr, &source, &board::now()) {
        Ok((slug, n, next)) => {
            writeln!(
                out,
                "rebuild=done slug={slug} stages={n} next={}",
                next.as_deref().unwrap_or("none")
            )?;
            Ok(exit::OK)
        }
        Err(rebuild::Error::Refused(r)) => refuse(out, &r.refusal()),
        Err(rebuild::Error::Sql(e)) => Err(internal(e)),
        Err(rebuild::Error::Board(e)) => Err(internal(e)),
        Err(rebuild::Error::Io(e)) => Err(e),
    }
}
