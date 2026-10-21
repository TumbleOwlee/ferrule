//! The gauntlet: the project's build, lint and test steps from
//! `ferrule.toml`, run in a run's worktree. Everything they print goes to
//! the run's `gauntlet.log`; the caller reads one status line. A pass is
//! cached under the worktree's tree and the configured steps, so the same
//! tree passes again without running anything. A worktree with changes
//! beyond `HEAD` is dirty: its pass is never cached.

use std::collections::hash_map::DefaultHasher;
use std::fs::File;
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use rusqlite::{Connection, OptionalExtension, params};

use crate::config::{Coverage, Step};
use crate::git;
use crate::refusal::Refusal;
use crate::store::Board;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Pass {
        cov: String,
        sha: String,
        cached: bool,
        /// The worktree differs from `HEAD`: the pass is not of the commit.
        dirty: bool,
    },
    Fail {
        step: String,
        sha: String,
        timeout: bool,
        log: PathBuf,
    },
    /// The worktree is missing.
    NoWorktree,
    /// `ferrule.toml` names no steps.
    NoSteps,
}

impl Outcome {
    /// The `<verb>=refused` line when nothing could run: no worktree or no
    /// steps; `None` for a pass or a failed step.
    #[must_use]
    pub fn refusal(&self, verb: &'static str) -> Option<Refusal> {
        match self {
            Outcome::NoWorktree => Some(
                Refusal::new(verb, "no-worktree")
                    .hint("create it with: ferrule gate approve <slug> gate2"),
            ),
            Outcome::NoSteps => Some(
                Refusal::new(verb, "no-steps").hint("add [[gauntlet.step]] tables to ferrule.toml"),
            ),
            Outcome::Pass { .. } | Outcome::Fail { .. } => None,
        }
    }
}

impl std::fmt::Display for Outcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Outcome::Pass {
                cov,
                sha,
                cached,
                dirty,
            } => {
                write!(f, "gauntlet=pass cov={cov} sha={sha}")?;
                if *cached {
                    write!(f, " cached")?;
                }
                if *dirty {
                    write!(f, " dirty")?;
                }
                Ok(())
            }
            Outcome::Fail {
                step,
                sha,
                timeout,
                log,
            } => {
                write!(f, "gauntlet=fail step={step} sha={sha}")?;
                if *timeout {
                    write!(f, " reason=timeout")?;
                }
                write!(f, " log={}", log.display())
            }
            Outcome::NoWorktree | Outcome::NoSteps => match self.refusal("gauntlet") {
                Some(r) => write!(f, "{r}"),
                None => Ok(()),
            },
        }
    }
}

/// The tree `git add -A` would stage in `wt`, through a copy of its index
/// so the worktree's own index stays untouched.
#[must_use]
pub fn tree(wt: &Path) -> Option<String> {
    let index = git::out(
        wt,
        &["rev-parse", "--path-format=absolute", "--git-path", "index"],
    )?;
    let tmp = tempfile_path(wt)?;
    if Path::new(&index).exists() {
        std::fs::copy(&index, &tmp).ok()?;
    }
    let with_index = |args: &[&str]| {
        Command::new("git")
            .arg("-C")
            .arg(wt)
            .args(args)
            .env("GIT_INDEX_FILE", &tmp)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok()
            .filter(|o| o.status.success())
    };
    let tree = with_index(&["add", "-A", "."])
        .and_then(|_| with_index(&["write-tree"]))
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned());
    let _ = std::fs::remove_file(&tmp);
    tree.filter(|t| !t.is_empty())
}

/// Whether the worktree holds anything `HEAD` does not: its [`tree`]
/// differs from `HEAD^{tree}`, or either cannot be read.
#[must_use]
pub fn dirty(wt: &Path) -> bool {
    differs_from_head(wt, tree(wt).as_deref())
}

fn differs_from_head(wt: &Path, tree: Option<&str>) -> bool {
    let head = git::out(wt, &["rev-parse", "HEAD^{tree}"]);
    head.is_none() || head.as_deref() != tree
}

/// The cache key: the tree and the steps, so changing `ferrule.toml`'s
/// steps misses.
fn cache_key(tree: &str, steps: &[Step]) -> String {
    let mut hasher = DefaultHasher::new();
    format!("{steps:?}").hash(&mut hasher);
    format!("{tree}:{:016x}", hasher.finish())
}

/// A fresh path in the repository's git directory for a scratch index.
fn tempfile_path(wt: &Path) -> Option<PathBuf> {
    let dir = git::out(wt, &["rev-parse", "--path-format=absolute", "--git-dir"])?;
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    Some(Path::new(&dir).join(format!("ferrule-index-{}-{nanos}", std::process::id())))
}

/// Line coverage from a `cargo llvm-cov` summary: the third percentage on
/// its last `TOTAL` line.
#[must_use]
pub fn coverage(output: &str) -> Option<String> {
    let total = output.lines().rev().find(|l| l.starts_with("TOTAL"))?;
    total
        .split_whitespace()
        .filter(|w| w.ends_with('%'))
        .nth(2)
        .map(str::to_owned)
}

enum StepResult {
    Ok,
    Failed,
    TimedOut,
}

/// Kills the step's whole process group; the child leads its own.
#[cfg(unix)]
fn kill_group(child: &std::process::Child) {
    let _ = Command::new("kill")
        .args(["-KILL", "--", &format!("-{}", child.id())])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[cfg(not(unix))]
fn kill_group(_: &std::process::Child) {}

/// Runs one step with its output appended to `log`, killed after its
/// timeout.
fn run_step(wt: &Path, step: &Step, log: &Path) -> std::io::Result<StepResult> {
    let mut file = File::options().append(true).create(true).open(log)?;
    writeln!(file, "### {}: {}", step.name, step.cmd.join(" "))?;
    let Some((program, args)) = step.cmd.split_first() else {
        return Ok(StepResult::Failed);
    };
    let mut command = Command::new(program);
    command
        .args(args)
        .current_dir(wt)
        .stdin(Stdio::null())
        .stdout(file.try_clone()?)
        .stderr(file);
    // Its own process group, so a timeout reaches what the step spawned.
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    let child = command.spawn();
    let Ok(mut child) = child else {
        return Ok(StepResult::Failed);
    };
    let deadline = Instant::now() + step.timeout;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(if status.success() {
                StepResult::Ok
            } else {
                StepResult::Failed
            });
        }
        if Instant::now() >= deadline {
            kill_group(&child);
            let _ = child.kill();
            let _ = child.wait();
            return Ok(StepResult::TimedOut);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Runs the gauntlet for `run` in `wt`, its log at `log`.
pub fn run(
    db: &Board,
    run: &str,
    wt: &Path,
    log: &Path,
    steps: &[Step],
    force: bool,
    now: &str,
) -> Result<Outcome, Box<dyn std::error::Error + Send + Sync>> {
    run_in(db.conn(), run, wt, log, steps, force, now)
}

/// [`run`] over a connection core code already holds.
pub(crate) fn run_in(
    conn: &Connection,
    run: &str,
    wt: &Path,
    log: &Path,
    steps: &[Step],
    force: bool,
    now: &str,
) -> Result<Outcome, Box<dyn std::error::Error + Send + Sync>> {
    if !wt.is_dir() {
        return Ok(Outcome::NoWorktree);
    }
    if steps.is_empty() {
        return Ok(Outcome::NoSteps);
    }
    let sha = git::out(wt, &["rev-parse", "--short", "HEAD"]).unwrap_or_default();
    let tree = tree(wt);
    let dirty = differs_from_head(wt, tree.as_deref());
    let key = tree.as_deref().map(|t| cache_key(t, steps));
    if let (Some(key), false, false) = (&key, force, dirty) {
        let cached: Option<String> = conn
            .query_row(
                "SELECT cov FROM gauntlet_cache WHERE run = ?1 AND tree = ?2",
                params![run, key],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(cov) = cached {
            return Ok(Outcome::Pass {
                cov,
                sha,
                cached: true,
                dirty,
            });
        }
    }
    if let Some(dir) = log.parent() {
        std::fs::create_dir_all(dir)?;
    }
    File::create(log)?;
    let mut cov = None;
    for step in steps {
        let before = std::fs::metadata(log)?.len();
        match run_step(wt, step, log)? {
            StepResult::Ok => {}
            r => {
                conn.execute("DELETE FROM gauntlet_cache WHERE run = ?1", params![run])?;
                return Ok(Outcome::Fail {
                    step: step.name.clone(),
                    sha,
                    timeout: matches!(r, StepResult::TimedOut),
                    log: log.to_path_buf(),
                });
            }
        }
        if step.coverage == Some(Coverage::LlvmCov) {
            let text = std::fs::read_to_string(log)?;
            let start = usize::try_from(before).unwrap_or(0).min(text.len());
            cov = coverage(text.get(start..).unwrap_or(&text));
        }
    }
    let cov = cov.unwrap_or_else(|| "?".to_owned());
    if let (Some(key), false) = (key, dirty) {
        conn.execute(
            "INSERT INTO gauntlet_cache (run, tree, cov, at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (run) DO UPDATE SET tree = excluded.tree, cov = excluded.cov, at = excluded.at",
            params![run, key, cov, now],
        )?;
    }
    Ok(Outcome::Pass {
        cov,
        sha,
        cached: false,
        dirty,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ut_coverage_is_the_third_percentage_of_the_last_total() {
        let out = "Filename  Regions Missed Cover Functions Missed Executed Lines Missed Cover\nTOTAL 100 20 80.00% 10 1 90.00% 500 94 81.20%\n";
        assert_eq!(coverage(out).as_deref(), Some("81.20%"));
        assert_eq!(coverage("nothing\n"), None);
    }

    #[cfg(unix)]
    #[test]
    fn ut_timeout_kills_the_steps_grandchildren() {
        let dir = tempfile::tempdir().unwrap();
        let pidfile = dir.path().join("pid");
        let step = Step {
            name: "slow".into(),
            cmd: vec![
                "sh".into(),
                "-c".into(),
                format!("sh -c 'echo $$ > {}; sleep 100' & wait", pidfile.display()),
            ],
            timeout: Duration::from_secs(1),
            coverage: None,
        };
        let log = dir.path().join("log");
        assert!(matches!(
            run_step(dir.path(), &step, &log).unwrap(),
            StepResult::TimedOut
        ));
        let pid = std::fs::read_to_string(&pidfile).unwrap();
        let alive = (0..20).any(|_| {
            std::thread::sleep(Duration::from_millis(50));
            Command::new("kill")
                .args(["-0", pid.trim()])
                .stderr(Stdio::null())
                .status()
                .unwrap()
                .success()
        });
        assert!(!alive, "grandchild {pid} survived the timeout");
    }
}
