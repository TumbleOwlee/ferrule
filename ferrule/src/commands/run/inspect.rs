//! What a run holds, printed: `run status`, `run diff`, `run log` and
//! `run plan-diff`. The answers come from `ferrule_core::run::inspect`.

use std::io::{self, Write};

use ferrule_core::config::Config;
use ferrule_core::git;
use ferrule_core::run::inspect::{self, Error, Invocation, Worktree};
use ferrule_core::store::Reader;

use crate::commands::board::store;
use crate::commands::{exit, internal, layout, refuse};

pub(super) use ferrule_core::run::inspect::Pick;

/// Prints a refusal, or passes an internal failure on.
fn failed(e: Error, out: &mut impl Write) -> io::Result<u8> {
    match e {
        Error::Refused(r) => refuse(out, &r),
        e => Err(internal(e)),
    }
}

/// Writes what each git command of `inv` prints, whole and in order.
fn write_git(inv: &Invocation, out: &mut impl Write) -> io::Result<u8> {
    for args in &inv.calls {
        out.write_all(&git::bytes(&inv.dir, args, &[]).map_err(internal)?)?;
    }
    Ok(exit::OK)
}

pub(super) fn diff(
    slug: &str,
    pick: &Pick,
    uncommitted: bool,
    out: &mut impl Write,
) -> io::Result<u8> {
    let diff = |r: Option<&Reader>| inspect::diff(r, &layout(), slug, pick, uncommitted);
    let inv = match store()? {
        Some(s) => s.read(|r| diff(Some(r))),
        None => diff(None),
    };
    match inv {
        Ok(inv) => write_git(&inv, out),
        Err(e) => failed(e, out),
    }
}

pub(super) fn log(slug: &str, pick: &Pick, out: &mut impl Write) -> io::Result<u8> {
    let log = |r: Option<&Reader>| inspect::log(r, &layout(), slug, pick);
    let inv = match store()? {
        Some(s) => s.read(|r| log(Some(r))),
        None => log(None),
    };
    match inv {
        Ok(inv) => write_git(&inv, out),
        Err(e) => failed(e, out),
    }
}

/// `-` for a value not set.
fn or_dash(v: Option<&str>) -> &str {
    v.unwrap_or("-")
}

pub(super) fn status(slug: &str, out: &mut impl Write) -> io::Result<u8> {
    let layout = layout();
    let config = Config::load(&layout.root).map_err(internal)?;
    let status = |r: Option<&Reader>| inspect::status(r, &layout, &config.main_branch, slug);
    let st = match store()? {
        Some(s) => s.read(|r| status(Some(r))),
        None => status(None),
    };
    let st = match st {
        Ok(st) => st,
        Err(e) => return failed(e, out),
    };
    let p = &st.parent;
    writeln!(
        out,
        "run={slug} state={} lane={} branch={} issue={} pr={} pr-state={} gates={}",
        p.state.as_str(),
        or_dash(p.lane.as_deref()),
        or_dash(p.branch.as_deref()),
        or_dash(p.issue.as_deref()),
        or_dash(p.pr.as_deref()),
        or_dash(p.pr_state.as_deref()),
        if p.gates.is_empty() {
            "-".to_owned()
        } else {
            p.gates.join(",")
        }
    )?;
    for stage in &st.stages {
        writeln!(
            out,
            "stage=s{} state={} commit={} title={}",
            stage.n,
            stage.state.as_str(),
            or_dash(stage.commit.as_deref()),
            or_dash(stage.title.as_deref())
        )?;
    }
    match &st.worktree {
        Worktree::Present {
            path,
            head,
            ahead,
            uncommitted,
        } => writeln!(
            out,
            "worktree={path} head={} ahead={} uncommitted={uncommitted}",
            or_dash(head.as_deref()),
            or_dash(ahead.as_deref()),
        )?,
        Worktree::Missing { path } => writeln!(out, "worktree=missing path={path}")?,
    }
    writeln!(out, "gauntlet={}", or_dash(st.gauntlet.as_deref()))?;
    writeln!(out, "ci={}", or_dash(st.ci.as_deref()))?;
    if let Some(l) = &st.last {
        writeln!(out, "last={} {} {}", l.card, l.at, l.body)?;
    }
    Ok(exit::OK)
}

pub(super) fn plan_diff(slug: &str, out: &mut impl Write) -> io::Result<u8> {
    let text = match inspect::plan_diff(&layout(), slug) {
        Ok(text) => text,
        Err(e) => return failed(e, out),
    };
    if text.is_empty() {
        writeln!(out, "plan-diff=same")?;
    } else {
        out.write_all(&text)?;
    }
    Ok(exit::OK)
}
