//! The gauntlet: the project's build, lint and test steps from
//! `ferrule.toml`, run in a run's worktree. Everything they print goes to
//! the run's `gauntlet.log`; the caller reads one status line. A pass is
//! cached under the worktree's tree and the configured steps, so the same
//! tree passes again without running anything. A worktree with changes
//! beyond `HEAD` is dirty: its pass is never cached.

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use rusqlite::{Connection, OptionalExtension, params};

use crate::config::{Coverage, Level, Step, default_timeout};
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
/// steps misses. The hash is ferrule's own, stable across Rust releases,
/// so a rebuilt binary still finds its cache.
fn cache_key(tree: &str, steps: &[Step]) -> String {
    format!("{tree}:{}", crate::install::hash(&format!("{steps:?}")))
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

pub(crate) enum StepResult {
    Ok,
    Failed,
    TimedOut,
}

/// Kills the step's whole process group; the child leads its own.
#[cfg(unix)]
pub(crate) fn kill_group(child: &std::process::Child) {
    let _ = Command::new("kill")
        .args(["-KILL", "--", &format!("-{}", child.id())])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// Kills the step's process tree.
#[cfg(windows)]
pub(crate) fn kill_group(child: &std::process::Child) {
    let _ = Command::new("taskkill")
        .args(["/T", "/F", "/PID", &child.id().to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn kill_group(_: &std::process::Child) {}

/// Runs one step with its output appended to `log`, killed after its
/// timeout.
pub(crate) fn run_step(wt: &Path, step: &Step, log: &Path) -> std::io::Result<StepResult> {
    let mut file = File::options().append(true).create(true).open(log)?;
    writeln!(file, "### {}: {}", step.name, step.cmd.join(" "))?;
    let Some((program, args)) = step.cmd.split_first() else {
        return Ok(StepResult::Failed);
    };
    let mut note = file.try_clone()?;
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
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(e) => {
            // The log is all the user is pointed at: it says why.
            writeln!(note, "ferrule: could not start {program}: {e}")?;
            return Ok(StepResult::Failed);
        }
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

/// What a gauntlet run covers.
#[derive(Debug, Clone, Copy)]
pub struct Steps<'a> {
    pub all: &'a [Step],
    pub level: Level,
    /// `--stage s<n>`: also run the tests this stage's diff adds or rewrites.
    pub stage: Option<u32>,
    /// The commit the stage's diff, and a review point's `{crates}`, start from.
    pub base: Option<&'a str>,
    pub stack: Option<&'a str>,
}

/// Package names of the crates whose files differ between `base` and the
/// worktree, untracked included.
#[must_use]
pub fn crates(wt: &Path, base: &str) -> Vec<String> {
    let mut paths: Vec<String> = git::out(wt, &["diff", "--name-only", "-z", base])
        .unwrap_or_default()
        .split('\0')
        .filter(|p| !p.is_empty())
        .map(str::to_owned)
        .collect();
    paths.extend(untracked(wt, &[]));
    let mut names: Vec<String> = paths
        .iter()
        .filter_map(|p| package_of(wt, Path::new(p)))
        .collect();
    names.sort();
    names.dedup();
    names
}

/// Untracked, not ignored paths of `wt` matching `specs` (all when empty).
fn untracked(wt: &Path, specs: &[&str]) -> Vec<String> {
    let mut args = vec!["ls-files", "-z", "--others", "--exclude-standard"];
    if !specs.is_empty() {
        args.push("--");
        args.extend(specs);
    }
    // `-z`: git quotes non-ASCII paths otherwise.
    git::out(wt, &args)
        .unwrap_or_default()
        .split('\0')
        .filter(|p| !p.is_empty())
        .map(str::to_owned)
        .collect()
}

/// The `[package] name` of the nearest `Cargo.toml` above `path`, up to `wt`.
fn package_of(wt: &Path, path: &Path) -> Option<String> {
    let mut dir = wt.join(path);
    while dir.pop() && dir.starts_with(wt) {
        if let Ok(text) = std::fs::read_to_string(dir.join("Cargo.toml"))
            && let Ok(manifest) = text.parse::<toml::Table>()
            && let Some(name) = manifest
                .get("package")
                .and_then(|p| p.get("name"))
                .and_then(toml::Value::as_str)
        {
            return Some(name.to_owned());
        }
    }
    None
}

/// The steps that run at `steps.level`, `{crates}` expanded.
fn selected(wt: &Path, steps: Steps<'_>) -> Vec<Step> {
    let rust = steps.stack == Some("rust");
    let crates = match (steps.level, steps.base) {
        (Level::Review, Some(base)) => crates(wt, base),
        _ => Vec::new(),
    };
    steps
        .all
        .iter()
        .filter(|s| steps.level == Level::Full || s.at.contains(&steps.level))
        .map(|s| {
            let mut step = s.clone();
            step.cmd = s
                .cmd
                .iter()
                .flat_map(|a| {
                    if a != "{crates}" {
                        vec![a.clone()]
                    } else if !rust {
                        Vec::new()
                    } else if crates.is_empty() {
                        vec!["--workspace".to_owned()]
                    } else {
                        crates
                            .iter()
                            .flat_map(|c| ["-p".to_owned(), c.clone()])
                            .collect()
                    }
                })
                .collect();
            step
        })
        .collect()
}

/// One step per test the stage's diff since `base` adds or rewrites.
fn stage_tests(wt: &Path, steps: Steps<'_>) -> Vec<Step> {
    let (Some(_), Some(base), Some(scanner)) = (
        steps.stage,
        steps.base,
        crate::run::testproof::scanner(steps.stack),
    ) else {
        return Vec::new();
    };
    let specs = scanner.paths();
    let mut args = vec!["diff", "--no-renames", "-U1000000", base, "--"];
    args.extend(specs);
    let mut text = git::out(wt, &args).unwrap_or_default();
    for path in untracked(wt, specs) {
        let added = git::bytes(
            wt,
            &[
                "diff",
                "--no-index",
                "--no-renames",
                "-U1000000",
                "--",
                "/dev/null",
                &path,
            ],
            &[1],
        )
        .unwrap_or_default();
        text.push('\n');
        text.push_str(&String::from_utf8_lossy(&added));
    }
    let changes = scanner.changes(&text);
    let mut names = changes.added;
    names.extend(changes.rewritten);
    names.sort();
    names.dedup();
    names
        .into_iter()
        .map(|name| Step {
            cmd: scanner.command(&name),
            name: format!("test:{name}"),
            timeout: default_timeout(),
            coverage: None,
            at: Vec::new(),
        })
        .collect()
}

/// The commit a run at `level` measures from: at push the base of `stage`,
/// at review the base of the first stage of the review point of the run's first
/// stage not yet done, none at full.
///
/// # Errors
///
/// The state database failed.
pub fn base(
    db: &Board,
    wt: &Path,
    run: &str,
    level: Level,
    stage: Option<u32>,
    main: &str,
) -> Result<Option<String>, crate::store::Error> {
    db.read(|r| {
        let conn = r.conn();
        let me = match level {
            Level::Push => stage,
            Level::Review => {
                let first = crate::card::board::stages(conn, run)?
                    .into_iter()
                    .filter(|(_, _, st)| *st != crate::card::board::State::Done)
                    .map(|(n, _, _)| n)
                    .min();
                match first {
                    Some(n) => Some(
                        match crate::run::tier::review_point(conn, wt, run, n, main)? {
                            crate::run::tier::ReviewPoint::Batch(v) => v[0],
                            crate::run::tier::ReviewPoint::Stage(_)
                            | crate::run::tier::ReviewPoint::None => n,
                        },
                    ),
                    None => None,
                }
            }
            Level::Full => None,
        };
        match me {
            Some(n) => Ok(crate::run::stage_base(conn, wt, run, n, main)?),
            None => Ok(None),
        }
    })
}

/// Runs the gauntlet for `run` in `wt`, its log at `log`; at the full level
/// on a clean worktree the result is logged on the run card.
pub fn run(
    db: &mut Board,
    run: &str,
    wt: &Path,
    log: &Path,
    steps: Steps<'_>,
    force: bool,
    now: &str,
) -> Result<Outcome, Box<dyn std::error::Error + Send + Sync>> {
    let outcome = run_in(db.conn(), run, wt, log, steps, force, now)?;
    if steps.level == Level::Full {
        match &outcome {
            Outcome::Pass {
                cov,
                sha,
                dirty: false,
                ..
            } => db.write(|w| {
                w.log(
                    run,
                    "gauntlet=pass",
                    &[format!("cov={cov}"), format!("sha={sha}")],
                    now,
                )
            })?,
            Outcome::Fail {
                step, sha, timeout, ..
            } if !dirty(wt) => {
                let mut args = vec![format!("step={step}"), format!("sha={sha}")];
                if *timeout {
                    args.push("reason=timeout".to_owned());
                }
                db.write(|w| w.log(run, "gauntlet=fail", &args, now))?;
            }
            _ => {}
        }
    }
    Ok(outcome)
}

/// [`run`] without the card log, over a connection core code already holds.
pub(crate) fn run_in(
    conn: &Connection,
    run: &str,
    wt: &Path,
    log: &Path,
    steps: Steps<'_>,
    force: bool,
    now: &str,
) -> Result<Outcome, Box<dyn std::error::Error + Send + Sync>> {
    if !wt.is_dir() {
        return Ok(Outcome::NoWorktree);
    }
    if steps.level == Level::Full && steps.all.is_empty() {
        return Ok(Outcome::NoSteps);
    }
    let sha = git::out(wt, &["rev-parse", "--short", "HEAD"]).unwrap_or_default();
    let tree = tree(wt);
    let dirty = differs_from_head(wt, tree.as_deref());
    // The new tests run after the level's steps; they join the cache key, so
    // a pass is reused only for the same tests.
    let mut chosen = selected(wt, steps);
    chosen.extend(stage_tests(wt, steps));
    let steps = chosen;
    if steps.is_empty() {
        return Ok(Outcome::Pass {
            cov: "?".to_owned(),
            sha,
            cached: false,
            dirty,
        });
    }
    let key = tree.as_deref().map(|t| cache_key(t, &steps));
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
    for step in &steps {
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
            // This step's bytes only; its output need not be UTF-8.
            let bytes = std::fs::read(log)?;
            let start = usize::try_from(before).unwrap_or(0).min(bytes.len());
            cov = coverage(&String::from_utf8_lossy(&bytes[start..]));
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

    #[test]
    fn ut_a_step_that_cannot_start_says_why_in_the_log() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("gauntlet.log");
        let step = Step {
            name: "nope".into(),
            cmd: vec!["ferrule-no-such-program".into()],
            timeout: Duration::from_secs(5),
            coverage: None,
            at: vec![Level::Full],
        };
        assert!(matches!(
            run_step(dir.path(), &step, &log).unwrap(),
            StepResult::Failed
        ));
        let text = std::fs::read_to_string(&log).unwrap();
        assert!(
            text.contains("could not start ferrule-no-such-program"),
            "{text}"
        );
    }

    #[test]
    fn ut_cache_key_is_stable() {
        let step = Step {
            name: "t".into(),
            cmd: vec!["true".into()],
            timeout: Duration::from_secs(5),
            coverage: None,
            at: vec![Level::Full],
        };
        let one = std::slice::from_ref(&step);
        assert_eq!(cache_key("abc", one), cache_key("abc", one));
        assert_eq!(
            cache_key("abc", one),
            format!("abc:{}", crate::install::hash(&format!("{one:?}")))
        );
        assert!(cache_key("abc", &[]).starts_with("abc:"));
    }

    fn step(name: &str, cmd: &[&str], at: &[Level]) -> Step {
        Step {
            name: name.into(),
            cmd: cmd.iter().map(|&c| c.to_owned()).collect(),
            timeout: Duration::from_secs(5),
            coverage: None,
            at: at.to_vec(),
        }
    }

    fn names(steps: &[Step]) -> Vec<&str> {
        steps.iter().map(|s| s.name.as_str()).collect()
    }

    fn all<'a>(
        steps: &'a [Step],
        level: Level,
        base: Option<&'a str>,
        stack: Option<&'a str>,
    ) -> Steps<'a> {
        Steps {
            all: steps,
            level,
            stage: None,
            base,
            stack,
        }
    }

    /// GA-R-082, GA-R-083, GA-R-084, GA-E-009 — a level selects the steps
    /// naming it; full selects every step.
    #[test]
    fn ut_selection_follows_the_level() {
        let steps = [
            step("p", &["a"], &[Level::Push]),
            step("r", &["b"], &[Level::Review]),
            step("pr", &["c"], &[Level::Push, Level::Review]),
            step("f", &["d"], &[Level::Full]),
        ];
        let dir = tempfile::tempdir().unwrap();
        let pick = |l| names(&selected(dir.path(), all(&steps, l, None, None))).join(",");
        assert_eq!(pick(Level::Push), "p,pr");
        assert_eq!(pick(Level::Review), "r,pr");
        assert_eq!(pick(Level::Full), "p,r,pr,f");
    }

    fn workspace() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let wt = dir.path();
        for (d, name) in [("a", "alpha"), ("b", "beta")] {
            std::fs::create_dir_all(wt.join(d).join("src")).unwrap();
            std::fs::write(
                wt.join(d).join("Cargo.toml"),
                format!("[package]\nname = \"{name}\"\nversion = \"0.0.0\"\n"),
            )
            .unwrap();
            std::fs::write(wt.join(d).join("src/lib.rs"), "\n").unwrap();
        }
        std::fs::write(
            wt.join("Cargo.toml"),
            "[workspace]\nmembers = [\"a\", \"b\"]\n",
        )
        .unwrap();
        git::run(wt, &["init", "-q"]).unwrap();
        git::run(wt, &["add", "."]).unwrap();
        git::run(
            wt,
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "commit",
                "-q",
                "-m",
                "x",
            ],
        )
        .unwrap();
        dir
    }

    /// GA-R-091 — `crates` names the packages of the tracked and untracked
    /// files that differ from the base.
    #[test]
    fn ut_crates_names_the_changed_packages() {
        let dir = workspace();
        let wt = dir.path();
        assert!(crates(wt, "HEAD").is_empty());
        std::fs::write(wt.join("a/src/lib.rs"), "// changed\n").unwrap();
        assert_eq!(crates(wt, "HEAD"), ["alpha"]);
        std::fs::write(wt.join("b/src/new.rs"), "\n").unwrap();
        std::fs::write(wt.join("README.md"), "\n").unwrap();
        assert_eq!(crates(wt, "HEAD"), ["alpha", "beta"]);
    }

    /// GA-R-091, GA-E-011 — at review `{crates}` is `-p <name>` per touched crate, the
    /// whole workspace when none is.
    #[test]
    fn ut_crates_expands_per_touched_crate_at_review() {
        let dir = workspace();
        let wt = dir.path();
        let steps = [step("t", &["cargo", "test", "{crates}"], &[Level::Review])];
        let go = |base| {
            selected(wt, all(&steps, Level::Review, base, Some("rust")))[0]
                .cmd
                .join(" ")
        };
        assert_eq!(go(Some("HEAD")), "cargo test --workspace");
        std::fs::write(wt.join("b/src/lib.rs"), "// changed\n").unwrap();
        assert_eq!(go(Some("HEAD")), "cargo test -p beta");
        assert_eq!(go(None), "cargo test --workspace");
    }

    /// GA-E-011 — `{crates}` is the whole workspace at a push and at full.
    #[test]
    fn ut_crates_expands_to_workspace_at_push_and_full() {
        let dir = workspace();
        let wt = dir.path();
        std::fs::write(wt.join("b/src/lib.rs"), "// changed\n").unwrap();
        let steps = [step("t", &["cargo", "test", "{crates}"], &[Level::Push])];
        for level in [Level::Push, Level::Full] {
            let got = selected(wt, all(&steps, level, Some("HEAD"), Some("rust")));
            assert_eq!(got[0].cmd.join(" "), "cargo test --workspace", "{level:?}");
        }
    }

    /// GA-E-011 — `{crates}` is dropped on a stack that is not Rust.
    #[test]
    fn ut_crates_dropped_off_rust() {
        let dir = workspace();
        let steps = [step("t", &["make", "{crates}", "x"], &[Level::Push])];
        for stack in [None, Some("node")] {
            let got = selected(dir.path(), all(&steps, Level::Push, Some("HEAD"), stack));
            assert_eq!(got[0].cmd, ["make", "x"]);
        }
    }

    /// Levels selecting different steps give
    /// different cache keys, so a pass at one level is not reused at another.
    #[test]
    fn ut_cache_key_differs_between_levels() {
        let steps = [
            step("p", &["x"], &[Level::Push]),
            step("r", &["y"], &[Level::Review]),
        ];
        let dir = tempfile::tempdir().unwrap();
        let at = |l| {
            let chosen = selected(dir.path(), all(&steps, l, None, None));
            cache_key("abc", &chosen)
        };
        assert_ne!(at(Level::Push), at(Level::Review));
        assert_ne!(at(Level::Push), at(Level::Full));
        assert_eq!(at(Level::Push), at(Level::Push));
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
            at: vec![Level::Full],
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
