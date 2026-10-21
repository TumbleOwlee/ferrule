//! `git` as a subprocess, for the read-only questions the run commands ask.

use std::ffi::OsStr;
use std::path::Path;
use std::process::{Command, Stdio};

/// The directory a command runs in: the project root, whose empty path
/// (the root is the current directory) is `.`.
#[must_use]
pub fn at(root: &Path) -> &Path {
    if root.as_os_str().is_empty() {
        Path::new(".")
    } else {
        root
    }
}

fn command<S: AsRef<OsStr>>(dir: &Path, args: &[S]) -> Command {
    let mut c = Command::new("git");
    c.arg("-C").arg(at(dir)).args(args).stdin(Stdio::null());
    c
}

/// A git command that failed, with what it said on stderr.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitError {
    pub args: Vec<String>,
    /// The exit code; `None` when git was killed or did not start.
    pub code: Option<i32>,
    pub stderr: String,
}

impl std::fmt::Display for GitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let why = self.stderr.lines().next().unwrap_or("").trim();
        write!(f, "git {}", self.args.join(" "))?;
        if why.is_empty() {
            match self.code {
                Some(c) => write!(f, ": exit {c}"),
                None => write!(f, ": no exit code"),
            }
        } else {
            write!(f, ": {why}")
        }
    }
}

impl std::error::Error for GitError {}

/// Stdout, trailing newline trimmed, of a git command that succeeded;
/// the exit code and stderr of one that did not.
///
/// # Errors
/// [`GitError`] when git did not start or exited non-zero.
pub fn run<S: AsRef<OsStr>>(dir: &Path, args: &[S]) -> Result<String, GitError> {
    let o = bytes(dir, args, &[])?;
    Ok(String::from_utf8_lossy(&o).trim_end().to_owned())
}

/// Stdout, byte for byte, of a git command that succeeded or exited with
/// one of `also_ok` (`diff --no-index` exits 1 when the files differ).
///
/// # Errors
/// [`GitError`] when git did not start or exited with another code.
pub fn bytes<S: AsRef<OsStr>>(
    dir: &Path,
    args: &[S],
    also_ok: &[i32],
) -> Result<Vec<u8>, GitError> {
    let fail = |code, stderr: String| GitError {
        args: args
            .iter()
            .map(|a| a.as_ref().to_string_lossy().into_owned())
            .collect(),
        code,
        stderr: stderr.trim_end().to_owned(),
    };
    let o = command(dir, args)
        .output()
        .map_err(|e| fail(None, e.to_string()))?;
    if o.status.success() || o.status.code().is_some_and(|c| also_ok.contains(&c)) {
        Ok(o.stdout)
    } else {
        Err(fail(
            o.status.code(),
            String::from_utf8_lossy(&o.stderr).into_owned(),
        ))
    }
}

/// Stdout, trailing newline trimmed, of a git command that succeeded.
#[must_use]
pub fn out<S: AsRef<OsStr>>(dir: &Path, args: &[S]) -> Option<String> {
    run(dir, args).ok()
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
pub fn ok<S: AsRef<OsStr>>(dir: &Path, args: &[S]) -> bool {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ut_run_carries_stderr_and_code_of_a_failing_command() {
        let dir = tempfile::tempdir().unwrap();
        assert!(ok(dir.path(), &["init", "-q"]));
        let e = run(dir.path(), &["rev-parse", "--verify", "nope"]).unwrap_err();
        assert_eq!(e.code, Some(128));
        assert!(
            e.stderr.contains("Needed a single revision"),
            "{}",
            e.stderr
        );
        assert!(
            e.to_string()
                .ends_with("nope: fatal: Needed a single revision")
        );
        assert_eq!(out(dir.path(), &["rev-parse", "--verify", "nope"]), None);
        let args = [OsStr::new("rev-parse"), OsStr::new("--git-dir")];
        assert_eq!(run(dir.path(), &args).unwrap(), ".git");
    }
}
