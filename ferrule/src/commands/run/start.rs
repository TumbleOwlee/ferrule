//! Starting a run and putting one back: `run init`, `run reconcile` and
//! `run rebuild`.

use std::io::{self, Write};
use std::path::PathBuf;

use ferrule_core::card::board;
use ferrule_core::config::Config;
use ferrule_core::remote::{self, Item};
use ferrule_core::run::{self as core_run, Error, gate, rebuild};

use crate::commands::board::{run_on, store};
use crate::commands::{exit, internal, layout, refuse};

pub(super) fn init(slug: &str, issue: Option<&str>, out: &mut impl Write) -> io::Result<u8> {
    let layout = layout();
    let config = Config::load(&layout.root).map_err(internal)?;
    let Some(mut s) = store(true)? else {
        return Ok(exit::REFUSED);
    };
    match gate::init(&mut s.conn, &layout, &config, slug, issue, &board::now()) {
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
    let mut store = store(false)?;
    let result = match &store {
        Some(s) => core_run::reconcile(&s.conn, &layout, slug),
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
    config: Config,
    pr: String,
    issue: String,
    root: PathBuf,
}

impl rebuild::Source for Remote {
    fn pr(&self) -> Option<Item> {
        remote::pr(self.config.backend.ci, &self.pr, &self.root).ok()
    }

    fn branch(&self) -> Option<String> {
        remote::pr_branch(self.config.backend.ci, &self.pr, &self.root).ok()
    }

    fn issue(&self) -> Option<Item> {
        remote::issue(self.config.backend.tracker, &self.issue, &self.root).ok()
    }
}

pub(super) fn rebuild(issue: &str, pr: &str, out: &mut impl Write) -> io::Result<u8> {
    let layout = layout();
    let config = Config::load(&layout.root).map_err(internal)?;
    let main = config.main_branch.clone();
    let source = Remote {
        config,
        pr: pr.to_owned(),
        issue: issue.to_owned(),
        root: layout.root.clone(),
    };
    let Some(mut s) = store(true)? else {
        return Ok(exit::REFUSED);
    };
    match rebuild::rebuild(
        &mut s.conn,
        &layout,
        &main,
        issue,
        pr,
        &source,
        &board::now(),
    ) {
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
