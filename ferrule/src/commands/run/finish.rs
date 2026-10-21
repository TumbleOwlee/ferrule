//! Ending a run: `run cleanup`, `run metrics` and `run export`.

use std::io::{self, Write};
use std::path::Path;

use ferrule_core::card::board;
use ferrule_core::project::DIR;
use ferrule_core::refusal::Refusal;
use ferrule_core::run::archive::{self, Error as ArchiveError};

use crate::commands::board::store;
use crate::commands::{exit, internal, layout, refuse};

/// `export=refused reason=no-archive`.
fn no_archive() -> Refusal {
    Refusal::new("export", "no-archive")
        .hint("the run has no such archive; `ferrule run cleanup` creates one and prints its id")
}

pub(super) fn metrics(
    slug: Option<&str>,
    last: Option<usize>,
    out: &mut impl Write,
) -> io::Result<u8> {
    let Some(mut s) = store()? else {
        if let Some(slug) = slug {
            eprintln!("metrics: no card for {slug}");
            return Ok(exit::REFUSED);
        }
        return Ok(exit::OK);
    };
    let Some(slug) = slug else {
        let mut lines = s.read(archive::metrics_log).map_err(internal)?;
        if let Some(n) = last {
            lines.drain(..lines.len().saturating_sub(n));
        }
        for line in lines {
            writeln!(out, "{line}")?;
        }
        return Ok(exit::OK);
    };
    match s.write(|w| archive::metrics(w, slug, &board::now())) {
        Ok(line) => {
            writeln!(out, "{line}")?;
            Ok(exit::OK)
        }
        Err(ArchiveError::Card) => {
            eprintln!("metrics: no card for {slug}");
            Ok(exit::REFUSED)
        }
        Err(ArchiveError::Ci(r)) => refuse(out, &r.refusal("metrics", slug)),
        Err(e) => Err(internal(e)),
    }
}

pub(super) fn cleanup(slug: &str, out: &mut impl Write) -> io::Result<u8> {
    let layout = layout();
    let Some(mut s) = store()? else {
        return refuse(out, &Refusal::card("cleanup"));
    };
    let refusal = match archive::cleanup(&mut s, &layout, slug, &board::now()) {
        Ok(done) => {
            let id = done
                .archive
                .map_or_else(|| "none".to_owned(), |i| i.to_string());
            let resumed = if done.resumed { " resumed" } else { "" };
            writeln!(out, "cleanup=done archive={id}{resumed}")?;
            return Ok(exit::OK);
        }
        Err(ArchiveError::Refused(r)) => r.refusal(slug),
        Err(ArchiveError::Ci(r)) => r.refusal("cleanup", slug),
        Err(ArchiveError::Card) => Refusal::card("cleanup"),
        Err(ArchiveError::Io(e)) => return Err(e),
        Err(ArchiveError::Sql(e)) => return Err(internal(e)),
    };
    refuse(out, &refusal)
}

pub(super) fn export(
    slug: &str,
    id: Option<i64>,
    to: Option<&Path>,
    out: &mut impl Write,
) -> io::Result<u8> {
    let layout = layout();
    let Some(s) = store()? else {
        return refuse(out, &no_archive());
    };
    let id = match id {
        Some(id) => Some(id),
        None => s
            .read(|r| archive::archives(r, slug))
            .map_err(internal)?
            .last()
            .map(|(id, _)| *id),
    };
    let Some(id) = id else {
        return refuse(out, &no_archive());
    };
    let dest = to.map_or_else(
        || {
            layout
                .root
                .join(DIR)
                .join("export")
                .join(format!("{slug}-{id}"))
        },
        Path::to_path_buf,
    );
    match s.read(|r| archive::export(r, slug, Some(id), &dest)) {
        Ok(Some((id, path))) => {
            writeln!(out, "export=done archive={id} path={}", path.display())?;
            Ok(exit::OK)
        }
        Ok(None) => refuse(out, &no_archive()),
        Err(ArchiveError::Io(e)) => Err(e),
        Err(e) => Err(internal(e)),
    }
}
