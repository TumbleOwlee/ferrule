use std::io::{self, Write};

use ferrule_core::board;
use ferrule_core::config::Config;
use ferrule_core::project::{self, Layout};
use ferrule_core::run::{self as core_run, Error};

use super::card::{sql, store};
use super::exit;
use crate::cli::RunCommand;

fn layout() -> Layout {
    Layout::new(project::main_checkout())
}

pub fn run(command: &RunCommand, out: &mut impl Write) -> io::Result<u8> {
    match command {
        RunCommand::ShaCheck { slug, stage, sha } => sha_check(slug, *stage, sha, out),
        RunCommand::Reconcile { slug } => reconcile(slug, out),
        RunCommand::StageStop {
            slug,
            stage,
            threads,
        } => stage_stop(slug, *stage, *threads, out),
    }
}

fn sha_check(slug: &str, stage: u32, sha: &str, out: &mut impl Write) -> io::Result<u8> {
    let verdict = match store(false)? {
        Some(s) => {
            core_run::sha_check(&s.conn, &layout(), slug, &format!("s{stage}"), sha).map_err(sql)?
        }
        None => Err(core_run::Mismatch::Card),
    };
    match verdict {
        Ok(()) => {
            writeln!(out, "sha=ok")?;
            Ok(exit::OK)
        }
        Err(m) => {
            writeln!(out, "sha=mismatch reason={}", m.as_str())?;
            Ok(exit::REFUSED)
        }
    }
}

fn reconcile(slug: &str, out: &mut impl Write) -> io::Result<u8> {
    let result = match store(false)? {
        Some(s) => core_run::reconcile(&s.conn, &layout(), slug),
        None => Err(Error::Card),
    };
    let r = match result {
        Ok(r) => r,
        Err(Error::Card) => {
            eprintln!("reconcile: no card for {slug}");
            return Ok(exit::USAGE);
        }
        Err(Error::Sql(e)) => return Err(sql(e)),
    };
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

fn stage_stop(slug: &str, stage: u32, threads: u32, out: &mut impl Write) -> io::Result<u8> {
    let layout = layout();
    let config = Config::load(&layout.root).map_err(sql)?;
    let Some(mut s) = store(false)? else {
        writeln!(out, "stage-stop=card")?;
        return Ok(exit::REFUSED);
    };
    match core_run::stage_stop(
        &mut s.conn,
        &layout,
        &config,
        slug,
        stage,
        threads,
        &board::now(),
    ) {
        Ok(Some(stop)) => writeln!(out, "{stop}")?,
        Ok(None) => writeln!(out, "auto")?,
        Err(Error::Card) => {
            writeln!(out, "stage-stop=card")?;
            return Ok(exit::REFUSED);
        }
        Err(Error::Sql(e)) => return Err(sql(e)),
    }
    Ok(exit::OK)
}
