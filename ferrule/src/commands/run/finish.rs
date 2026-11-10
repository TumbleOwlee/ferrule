//! Ending a run: `run cleanup`, `run metrics` and `run export`.

use std::io::{self, Write};
use std::path::Path;

use ferrule_core::card::board;
use ferrule_core::project::DIR;
use ferrule_core::refusal::Refusal;
use ferrule_core::remote;
use ferrule_core::run::abandon;
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
        if slug.is_some() {
            return refuse(out, &Refusal::card("metrics"));
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
        Err(ArchiveError::Card) => refuse(out, &Refusal::card("metrics")),
        Err(ArchiveError::Ci(r)) => refuse(out, &r.refusal("metrics", slug)),
        Err(e) => Err(internal(e)),
    }
}

pub(super) fn cleanup(slug: &str, out: &mut impl Write) -> io::Result<u8> {
    let layout = layout();
    let Some(mut s) = store()? else {
        return refuse(out, &Refusal::card("cleanup"));
    };
    let config = crate::commands::config()?;
    let tracker = remote::issue_tracker(&config, &layout.root);
    let refusal = match archive::cleanup(&mut s, &layout, slug, &board::now(), &*tracker) {
        Ok(done) => {
            let id = done
                .archive
                .map_or_else(|| "none".to_owned(), |i| i.to_string());
            let resumed = if done.resumed { " resumed" } else { "" };
            let issue = done
                .issue
                .map_or_else(String::new, |i| format!(" issue={i}"));
            writeln!(out, "cleanup=done archive={id}{resumed}{issue}")?;
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

pub(super) fn abandon(slug: &str, confirm: bool, out: &mut impl Write) -> io::Result<u8> {
    let layout = layout();
    let Some(mut s) = store()? else {
        return refuse(out, &Refusal::card("abandon"));
    };
    let config = crate::commands::config()?;
    let host = remote::code_host(&config, &layout.root);
    if !confirm {
        return match s.read(|r| abandon::plan(r, &layout, slug, &*host)) {
            Ok(Ok(plan)) => {
                if let Some(id) = plan.archived
                    && plan.worktree.is_none()
                    && plan.local_branch.is_none()
                    && !plan.run_dir
                {
                    writeln!(out, "abandon=already archive={id}")?;
                    return Ok(exit::OK);
                }
                writeln!(out, "abandon=plan")?;
                for path in &plan.uncommitted {
                    writeln!(out, "uncommitted {path}")?;
                }
                for commit in &plan.unpushed {
                    writeln!(out, "unpushed {commit}")?;
                }
                if let Some(pr) = &plan.pr {
                    writeln!(out, "close-pr {pr}")?;
                }
                if let Some(b) = &plan.remote_branch {
                    writeln!(out, "delete-remote-branch {b}")?;
                }
                if let Some(b) = &plan.local_branch {
                    writeln!(out, "delete-branch {b}")?;
                }
                if let Some(wt) = &plan.worktree {
                    writeln!(out, "remove-worktree {}", wt.display())?;
                }
                writeln!(
                    out,
                    "hint=on the user's word only: ferrule run abandon {slug} --confirm"
                )?;
                Ok(exit::OK)
            }
            Ok(Err(r)) => refuse(out, &r.refusal(slug)),
            Err(ArchiveError::Io(e)) => Err(e),
            Err(e) => Err(internal(e)),
        };
    }
    match abandon::abandon(&mut s, &layout, slug, &board::now(), &*host) {
        Ok(Ok(done)) => {
            let id = done
                .archive
                .map_or_else(|| "none".to_owned(), |i| i.to_string());
            if done.already {
                writeln!(out, "abandon=already archive={id}")?;
                return Ok(exit::OK);
            }
            let issue = done
                .issue
                .map_or_else(String::new, |i| format!(" issue={i}"));
            let resumed = if done.resumed { " resumed" } else { "" };
            writeln!(out, "abandon=done archive={id}{issue}{resumed}")?;
            Ok(exit::OK)
        }
        Ok(Err(r)) => refuse(out, &r.refusal(slug)),
        Err(ArchiveError::Io(e)) => Err(e),
        Err(e) => Err(internal(e)),
    }
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
