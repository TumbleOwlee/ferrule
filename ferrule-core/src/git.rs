//! `git` as a subprocess, for the read-only questions the run commands ask.

use std::path::Path;
use std::process::{Command, Stdio};

fn command(dir: &Path, args: &[&str]) -> Command {
    let mut c = Command::new("git");
    c.arg("-C").arg(dir).args(args).stdin(Stdio::null());
    c
}

/// Stdout, trailing newline trimmed, of a git command that succeeded.
#[must_use]
pub fn out(dir: &Path, args: &[&str]) -> Option<String> {
    let o = command(dir, args).stderr(Stdio::null()).output().ok()?;
    o.status
        .success()
        .then(|| String::from_utf8_lossy(&o.stdout).trim_end().to_owned())
}

/// Stdout, trailing newline trimmed, of `git grep <args>`: empty when
/// nothing matches (exit 1), `None` when git failed.
#[must_use]
pub fn grep(dir: &Path, args: &[&str]) -> Option<String> {
    let o = command(dir, &[&["grep"], args].concat())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    match o.status.code() {
        Some(0) => Some(String::from_utf8_lossy(&o.stdout).trim_end().to_owned()),
        Some(1) => Some(String::new()),
        _ => None,
    }
}

/// Whether a git command succeeded.
#[must_use]
pub fn ok(dir: &Path, args: &[&str]) -> bool {
    command(dir, args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// The full sha of the commit `rev` names, if it names one.
#[must_use]
pub fn commit(dir: &Path, rev: &str) -> Option<String> {
    out(
        dir,
        &["rev-parse", "-q", "--verify", &format!("{rev}^{{commit}}")],
    )
    .filter(|s| !s.is_empty())
}

/// Whether `rev` is an ancestor of (or is) `of`.
#[must_use]
pub fn is_ancestor(dir: &Path, rev: &str, of: &str) -> bool {
    ok(dir, &["merge-base", "--is-ancestor", rev, of])
}
