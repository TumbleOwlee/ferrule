//! Whether a run's branch has diverged from its remote head: the remote
//! head is not an ancestor of the local one.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crate::git::{self, GitError};
use crate::run::gauntlet;

/// How long the remote may take to answer.
const LIMIT: Duration = Duration::from_mins(1);

/// A branch whose remote head is not an ancestor of its local head.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diverged {
    /// The local head, short sha.
    pub local: String,
    /// The remote head, short sha.
    pub remote: String,
}

/// The divergence of `branch` in `dir` from `origin`, or `None` when it
/// has not diverged or the check cannot complete.
#[must_use]
pub fn check(dir: &Path, branch: &str) -> Option<Diverged> {
    check_within(dir, branch, LIMIT)
}

/// The reconcile row for `branch` in the checkout at `root`, when it has
/// diverged.
#[must_use]
pub fn row(root: &Path, branch: &str) -> Option<String> {
    let d = check(root, branch)?;
    Some(format!(
        "diverged {branch} local={} remote={}",
        d.local, d.remote
    ))
}

/// Where `branch` stands on `origin`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Remote {
    /// The branch does not exist on origin.
    Missing,
    /// The branch's head there, full sha.
    At(String),
}

/// The head of `branch` on `origin`, or `None` when the check cannot
/// complete.
#[must_use]
pub fn remote_head(dir: &Path, branch: &str) -> Option<Remote> {
    let name = format!("refs/heads/{branch}");
    let listing = bounded(dir, &["ls-remote", "origin", &name], Instant::now() + LIMIT)?;
    Some(
        listing
            .lines()
            .find_map(|line| {
                let (sha, found) = line.split_once('\t')?;
                (found.trim() == name).then(|| Remote::At(sha.to_owned()))
            })
            .unwrap_or(Remote::Missing),
    )
}

fn check_within(dir: &Path, branch: &str, limit: Duration) -> Option<Diverged> {
    let name = format!("refs/heads/{branch}");
    let local = git::commit(dir, &name)?;
    let deadline = Instant::now() + limit;
    let listing = bounded(
        dir,
        &["ls-remote", "--exit-code", "origin", &name],
        deadline,
    )?;
    let remote = listing.lines().find_map(|line| {
        let (sha, found) = line.split_once('\t')?;
        (found.trim() == name).then(|| sha.to_owned())
    })?;
    bounded(dir, &["fetch", "-q", "origin", &name], deadline)?;
    let ancestry = git::run(dir, &["merge-base", "--is-ancestor", &remote, &local]);
    if !is_diverged(&ancestry) {
        return None;
    }
    let short = |sha: &str| git::out(dir, &["rev-parse", "--short", sha]);
    Some(Diverged {
        local: short(&local)?,
        remote: short(&remote)?,
    })
}

/// Whether the result of `merge-base --is-ancestor` says "not an ancestor":
/// exit code 1, never git's error text.
fn is_diverged(result: &Result<String, GitError>) -> bool {
    matches!(result, Err(e) if e.code == Some(1))
}

/// Stdout of a git command that exits zero before `deadline`, run with
/// the credential prompt off in its own process group; `None` on any
/// failure or when waiting or reading passes the deadline (the group is
/// then killed).
fn bounded(dir: &Path, args: &[&str], deadline: Instant) -> Option<String> {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(git::at(dir))
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    let mut child = command.spawn().ok()?;
    let mut pipe = child.stdout.take()?;
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = pipe.read_to_end(&mut bytes);
        let _ = tx.send(bytes);
    });
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => {
                let left = deadline.saturating_duration_since(Instant::now());
                let Ok(bytes) = rx.recv_timeout(left) else {
                    gauntlet::kill_group(&child);
                    return None;
                };
                return Some(String::from_utf8_lossy(&bytes).into_owned());
            }
            Ok(Some(_)) => return None,
            Ok(None) if Instant::now() < deadline => {}
            _ => {
                gauntlet::kill_group(&child);
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant};

    use super::*;

    fn g(dir: &Path, args: &[&str]) -> String {
        let mut all = vec![
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
        ];
        all.extend_from_slice(args);
        git::run(dir, &all).unwrap_or_else(|e| panic!("{e}"))
    }

    fn commit(dir: &Path, file: &str) {
        std::fs::write(dir.join(file), file).unwrap();
        g(dir, &["add", file]);
        g(dir, &["commit", "-q", "-m", file]);
    }

    /// A bare origin and a clone `a` of it holding `main` at one commit, pushed.
    fn setup() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let origin = tmp.path().join("origin.git");
        std::fs::create_dir(&origin).unwrap();
        g(&origin, &["init", "-q", "--bare", "-b", "main"]);
        let a = tmp.path().join("a");
        g(tmp.path(), &["clone", "-q", origin.to_str().unwrap(), "a"]);
        g(&a, &["checkout", "-q", "-b", "main"]);
        commit(&a, "one");
        g(&a, &["push", "-q", "origin", "main"]);
        (tmp, origin, a)
    }

    fn clone_of(tmp: &Path, origin: &Path, name: &str) -> PathBuf {
        g(tmp, &["clone", "-q", origin.to_str().unwrap(), name]);
        tmp.join(name)
    }

    fn short(dir: &Path) -> String {
        g(dir, &["rev-parse", "--short", "HEAD"])
    }

    /// The git pin: `ls-remote --exit-code` exits 2 for a missing branch.
    #[test]
    fn ut_ls_remote_exits_two_for_a_missing_branch() {
        let tmp = tempfile::tempdir().unwrap();
        let origin = tmp.path().join("origin.git");
        std::fs::create_dir(&origin).unwrap();
        g(&origin, &["init", "-q", "--bare"]);
        let a = tmp.path().join("a");
        g(tmp.path(), &["clone", "-q", origin.to_str().unwrap(), "a"]);
        commit(&a, "one");
        let err = git::run(
            &a,
            &["ls-remote", "--exit-code", "origin", "refs/heads/nope"],
        )
        .unwrap_err();
        assert_eq!(err.code, Some(2));
        let branch = g(&a, &["symbolic-ref", "--short", "HEAD"]);
        assert_eq!(check(&a, &branch), None);
    }

    /// GW-R-008 — `merge-base --is-ancestor` exits 1 for a non-ancestor.
    #[test]
    fn ut_merge_base_exits_one_for_a_non_ancestor() {
        let (_t, _o, a) = setup();
        let first = g(&a, &["rev-parse", "HEAD"]);
        commit(&a, "two");
        let second = g(&a, &["rev-parse", "HEAD"]);
        let err = git::run(&a, &["merge-base", "--is-ancestor", &second, &first]).unwrap_err();
        assert_eq!(err.code, Some(1));
        assert!(is_diverged(&Err(err)));
    }

    /// GW-R-008 — both sides committed: the diverged branch is reported by short sha.
    #[test]
    fn ut_check_reports_short_shas_of_a_diverged_branch() {
        let (t, o, a) = setup();
        let b = clone_of(t.path(), &o, "b");
        commit(&a, "two");
        g(&a, &["push", "-q", "origin", "main"]);
        commit(&b, "three");
        let found = check(&b, "main").unwrap();
        assert_eq!(found.local, short(&b));
        assert_eq!(found.remote, short(&a));
    }

    /// GW-R-018 — the row names the branch and both short heads.
    #[test]
    fn ut_row_formats_both_heads() {
        let (t, o, a) = setup();
        let b = clone_of(t.path(), &o, "b");
        commit(&a, "two");
        g(&a, &["push", "-q", "origin", "main"]);
        commit(&b, "three");
        let want = format!("diverged main local={} remote={}", short(&b), short(&a));
        assert_eq!(row(&b, "main"), Some(want));
        assert_eq!(row(&a, "main"), None);
    }

    #[test]
    fn ut_check_reports_a_branch_only_behind_the_remote() {
        let (t, o, a) = setup();
        let b = clone_of(t.path(), &o, "b");
        commit(&a, "two");
        g(&a, &["push", "-q", "origin", "main"]);
        let found = check(&b, "main").unwrap();
        assert_eq!(found.local, short(&b));
        assert_eq!(found.remote, short(&a));
    }

    #[test]
    fn ut_check_is_clear_when_the_remote_is_an_ancestor() {
        let (_t, _o, a) = setup();
        assert_eq!(check(&a, "main"), None);
        commit(&a, "two");
        assert_eq!(check(&a, "main"), None);
    }

    #[test]
    fn ut_check_is_clear_for_an_unreadable_remote() {
        let (t, _o, a) = setup();
        let gone = t.path().join("gone");
        g(&a, &["remote", "set-url", "origin", gone.to_str().unwrap()]);
        assert_eq!(check(&a, "main"), None);
    }

    /// GW-R-008 — only exit 1 is divergence.
    #[test]
    fn ut_check_is_clear_when_merge_base_fails_otherwise() {
        let err = |code| GitError {
            args: vec![],
            code,
            stderr: String::new(),
        };
        assert!(is_diverged(&Err(err(Some(1)))));
        assert!(!is_diverged(&Err(err(Some(128)))));
        assert!(!is_diverged(&Err(err(None))));
        assert!(!is_diverged(&Ok(String::new())));
    }

    /// `ls-remote` also lists `refs/heads/a/refs/heads/dev`, sorted before
    /// the exact ref: only the exact ref's head counts.
    #[test]
    fn ut_check_matches_the_whole_ref() {
        let (_t, o, a) = setup();
        let other = tempfile::tempdir().unwrap();
        let p = other.path().join("p");
        std::fs::create_dir(&p).unwrap();
        g(&p, &["init", "-q", "-b", "main"]);
        commit(&p, "unrelated");
        g(
            &p,
            &[
                "push",
                "-q",
                o.to_str().unwrap(),
                "main:refs/heads/a/refs/heads/dev",
            ],
        );
        g(&a, &["push", "-q", "origin", "main:refs/heads/dev"]);
        g(
            &a,
            &["fetch", "-q", "origin", "refs/heads/a/refs/heads/dev"],
        );
        g(&a, &["branch", "-q", "dev"]);
        g(&a, &["checkout", "-q", "dev"]);
        commit(&a, "two");
        assert_eq!(check(&a, "dev"), None);
    }

    /// Points `origin` of `repo` at an ssh url run by `script`.
    #[cfg(unix)]
    fn ssh_origin(repo: &Path, script: &str) {
        let path = repo.join("..").join("ssh.sh");
        std::fs::write(&path, script).unwrap();
        g(repo, &["remote", "set-url", "origin", "ssh://x/y"]);
        g(
            repo,
            &[
                "config",
                "core.sshCommand",
                &format!("sh {}", path.display()),
            ],
        );
    }

    #[cfg(unix)]
    fn hold(repo: &Path, body: &str) {
        g(
            repo,
            &["config", "alias.hold", &format!("!f() {{ {body}; }}; f")],
        );
    }

    #[cfg(unix)]
    fn running(pid: &str) -> bool {
        let state = if Path::new("/proc").is_dir() {
            let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
                return false;
            };
            let Some((_, rest)) = stat.rsplit_once(')') else {
                return false;
            };
            rest.trim_start().chars().next()
        } else {
            let out = Command::new("ps")
                .args(["-o", "stat=", "-p", pid])
                .output()
                .unwrap();
            String::from_utf8_lossy(&out.stdout).trim().chars().next()
        };
        state.is_some_and(|c| c != 'Z')
    }

    #[cfg(unix)]
    fn gone(pid: &str) -> bool {
        for _ in 0..100 {
            if !running(pid) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        false
    }

    /// GW-R-029 — git exited, its helper holds the output past the limit: the helper is killed.
    #[cfg(unix)]
    #[test]
    fn ut_bounded_kills_a_helper_holding_output_past_the_limit() {
        let (t, _o, a) = setup();
        let file = t.path().join("pid");
        hold(&a, &format!("sleep 30 & echo $! > '{}'", file.display()));
        let start = Instant::now();
        let out = bounded(&a, &["hold"], Instant::now() + Duration::from_secs(1));
        let took = start.elapsed();
        let pid = std::fs::read_to_string(&file).unwrap();
        let pid = pid.trim();
        assert!(!pid.is_empty(), "pid file is empty");
        let reaped = gone(pid);
        if !reaped {
            let _ = Command::new("kill").args(["-KILL", pid]).status();
        }
        assert_eq!(out, None);
        assert!(took < Duration::from_secs(10), "{took:?}");
        assert!(reaped, "helper {pid} still running");
    }

    /// GW-R-022, GW-R-029 — git still running at the limit: every process is killed.
    #[cfg(unix)]
    #[test]
    fn ut_bounded_kills_every_process_when_git_outlives_the_limit() {
        let (t, _o, a) = setup();
        let file = t.path().join("pid");
        hold(
            &a,
            &format!("sleep 30 & echo $! > '{}'; wait", file.display()),
        );
        let start = Instant::now();
        let out = bounded(&a, &["hold"], Instant::now() + Duration::from_secs(1));
        let took = start.elapsed();
        let pid = std::fs::read_to_string(&file).unwrap();
        let pid = pid.trim();
        assert!(!pid.is_empty(), "pid file is empty");
        let reaped = gone(pid);
        if !reaped {
            let _ = Command::new("kill").args(["-KILL", pid]).status();
        }
        assert_eq!(out, None);
        assert!(took < Duration::from_secs(10), "{took:?}");
        assert!(reaped, "process {pid} still running");
    }

    /// GW-E-007 — a check finished in time leaves git's helper running.
    #[cfg(unix)]
    #[test]
    fn ut_bounded_leaves_a_helper_of_a_check_finished_in_time() {
        struct Reap(String);
        impl Drop for Reap {
            fn drop(&mut self) {
                let _ = Command::new("kill").args(["-KILL", &self.0]).status();
            }
        }
        let (t, _o, a) = setup();
        let file = t.path().join("pid");
        hold(
            &a,
            &format!(
                "sleep 30 >/dev/null 2>&1 & echo $! > '{}'; echo done",
                file.display()
            ),
        );
        let out = bounded(&a, &["hold"], Instant::now() + Duration::from_secs(10));
        let pid = std::fs::read_to_string(&file).unwrap();
        let _reap = Reap(pid.trim().to_owned());
        assert_eq!(out, Some("done\n".to_owned()));
        assert!(running(pid.trim()));
    }

    /// GW-R-022 — a check unfinished at its limit gives up.
    #[cfg(unix)]
    #[test]
    fn ut_check_gives_up_at_its_limit() {
        assert_eq!(LIMIT, Duration::from_mins(1));
        let (_t, _o, a) = setup();
        ssh_origin(&a, "exec sleep 30\n");
        let start = Instant::now();
        assert_eq!(check_within(&a, "main", Duration::from_secs(1)), None);
        let took = start.elapsed();
        assert!(
            took >= Duration::from_secs(1) && took < Duration::from_secs(10),
            "{took:?}"
        );
    }

    /// GW-R-025 — the check turns the credential prompt off.
    #[cfg(unix)]
    #[test]
    fn ut_check_turns_off_the_credential_prompt() {
        let (t, _o, a) = setup();
        let seen = t.path().join("seen");
        ssh_origin(
            &a,
            &format!(
                "printf %s \"$GIT_TERMINAL_PROMPT\" > {}\nexit 1\n",
                seen.display()
            ),
        );
        assert_eq!(check(&a, "main"), None);
        assert_eq!(std::fs::read_to_string(&seen).unwrap(), "0");
    }
}
