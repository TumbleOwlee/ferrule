use std::io::{self, Write};
use std::path::{Path, PathBuf};

use clap::{Args, Subcommand, ValueEnum};
use ferrule_core::card::board;
use ferrule_core::card::typed::Name;
use ferrule_core::config::Config;
use ferrule_core::git::GitError;
use ferrule_core::pr::publish;
use ferrule_core::project::{self, Layout};
use ferrule_core::refusal::{CARD_HINT, Refusal};
use ferrule_core::remote::{self, Item};
use ferrule_core::run::archive::{self, Error as ArchiveError};
use ferrule_core::run::gate::{self, Gate};
use ferrule_core::run::gauntlet::{self, Outcome};
use ferrule_core::run::next::{self, At, Status};
use ferrule_core::run::rebuild;
use ferrule_core::run::{self as core_run, Error};
use ferrule_core::store::DIR;

use super::board::{card_store, store};
use super::exit;
use super::{internal, refuse};

/// Shown after `ferrule run init --help`.
const INIT_AFTER_LONG_HELP: &str = "\
Output:
  init=done slug=<slug>            The card and .ferrule/runs/<slug>/ exist
  init=done slug=<slug> kind=slice epic=<n>
                                   --issue named a slice of epic <n>; the card
                                   records kind and epic
With --issue the run directory also holds request.md and issue-spec.md (when
the issue carries them) and, for a slice, slice-line.md and epic.md.
Refusals:
  init=refused reason=exists       The run already has a card or stage cards;
                                   resume it with `ferrule run reconcile <slug>`Errors (stderr):
  init: <host error>               The issue could not be read (exit 1)
Refusals end in ` hint=<fix>`.
Exit codes: 0 done, 1 refused, 2 usage, 4 internal error.
See: ferrule guide show gate1";

/// Shown after `ferrule run reconcile --help`.
const RECONCILE_AFTER_LONG_HELP: &str = "\
Card = intent, git and the host = fact. Output: one row per disagreement,
then a summary:
  differ row=<what>                A card and git or the host disagree. Rows:
    worktree <path> missing        gate2 is set but the worktree is gone
    branch <b> missing             The run's branch does not exist
    commit s<n> <sha> does not exist
    commit s<n> <sha> not on <b>
    done s<n> has no commit        A done stage never logged commit=
    gauntlet <sha> not on <b>      The last gauntlet=pass is not on the branch
    issue #<n> ..., pr #<n> ...    The issue is not open or lacks comments, the
                                   PR is closed, merged or behind the board
                                   (Jira: a status of the `done` category is closed)
    host unreadable: <error>       The host could not be read
  reconcile=clean next=<s<n>|none>
                                   Nothing differs; `next` is the first stage
                                   not done
  reconcile=differ n=<rows> next=<s<n>|none>
                                   Fix the rows (or ask the user) before
                                   resuming
Errors (stderr):
  reconcile: no card for <slug>    Exit 2; check the slug with
                                   `ferrule board list`
Exit codes: 0 clean, 1 differ, 2 usage, 4 internal error.
See: ferrule guide show resume";

/// Shown after `ferrule run rebuild --help`.
const REBUILD_AFTER_LONG_HELP: &str = "\
Output:
  rebuild=done slug=<slug> stages=<n> next=<s<n>|none>
      The board is back: the slug comes from the PR's branch, the stage cards
      from its `## Stages` lines (a ticked line is a done stage with its
      commit), the worktree is re-added and the run files are rewritten
      (never plan.md). The parent logs `rebuild reason=board-missing` and
      `pr=draft`; each done stage logs its `commit=<sha>`. `next` is the
      first stage not done; run `ferrule run reconcile <slug>` before resuming.
Refusals (nothing is written):
  rebuild=refused reason=no-pr         The PR could not be read
  rebuild=refused reason=no-stages     The PR body has no `## Stages` list
  rebuild=refused reason=no-branch     The PR's branch is unknown or cannot be
                                       fetched from origin
  rebuild=refused reason=board-exists  The run still has a card, stage cards
                                       or run directory; use reconcile
  rebuild=refused reason=no-issue      The tracking issue could not be read
  rebuild=refused reason=sha sha=<sha> A ticked stage's commit is not in the
                                       repository
  rebuild=refused reason=no-sha stage=s<n>
                                       A ticked stage line has no `(<sha>)`
  rebuild=refused reason=bad-branch branch=<branch>
                                       The branch is not `<prefix>/<slug>`; the
                                       slug (after the first `/`) must be
                                       non-empty and hold no slash
  rebuild=refused reason=worktree      `git worktree add` failed
Refusals end in ` hint=<fix>`.
Exit codes: 0 done, 1 refused, 2 usage, 4 internal error.
See: ferrule guide show resume";

/// Shown after `ferrule run cleanup --help`.
const CLEANUP_AFTER_LONG_HELP: &str = "\
Output:
  cleanup=done archive=<id>        The metrics line is recorded and the cards
                                   and run files are archived as <id>
                                   (`ferrule run export` writes it back), in
                                   one transaction; then the worktree, local
                                   branch and run directory are removed
  cleanup=done archive=none        Nothing was left to archive
  cleanup=done archive=<id> resumed
                                   An earlier cleanup archived the run but
                                   failed removing the worktree, branch or
                                   run directory; this one finished those
Refusals before the archive is written (nothing is changed):
  cleanup=refused reason=card                 No such run
  cleanup=refused reason=detached-head        The worktree is on no branch
  cleanup=refused reason=uncommitted-changes  The worktree has changes; commit
                                              them, or ask the user, then push
  cleanup=refused reason=no-upstream          The branch was never pushed
  cleanup=refused reason=unpushed-commits     Push with `ferrule pr push <slug>`
  cleanup=refused reason=no-ci                No ci= logged; `ferrule ci wait`
  cleanup=refused reason=ci-red               The last ci= is not 0 and no later
                                              ci-waived line exists
Refusals after it (the run is archived; rerun to finish):
  cleanup=refused reason=worktree-remove-failed
                                              git could not remove the worktree;
                                              ask the user (the shell guard
                                              refuses `git worktree remove`)
  cleanup=refused reason=branch-delete-failed The local branch could not be
                                              deleted; `git branch -D` it
The two git failures carry ` detail=\"<git's error>\"`. Refusals end in
` hint=<fix>`.
Exit codes: 0 done, 1 refused, 4 internal error.
See: ferrule guide show cleanup";

/// Shown after `ferrule run metrics --help`.
const METRICS_AFTER_LONG_HELP: &str = "\
Output, one line (recorded; a rerun replaces the run's line):
  <slug> start=<t> end=<t> first-red=<t|-> stages=<n> stops=<n> question=<n>
  approval=<n> stage=<n> blocked=<n> replans=<n> overturned=<n> reads=<n>
  lane=<lane> ci=<code>
    start, end       First logged event and now; first-red the first test-red
    stops            `stop kind=` lines on the parent; question, approval,
                     stage and blocked count them by kind
    replans          `replan from=` lines; overturned sums `count=` of the
                     `overturned` lines; reads counts `read path=` lines
    lane, ci         lane is `epic` for an epic; ci is 0, `waived`, or `-` for
                     an epic
Without a slug: every recorded line, oldest run first (nothing if none);
--last <n> keeps the latest n.
Refusals:
  metrics=refused reason=no-ci     No ci= logged; run `ferrule ci wait <slug>`
  metrics=refused reason=ci-red    The last ci= is not 0 and no later
                                   ci-waived line exists
  (stderr) metrics: no card for <slug>
                                   Exit 1; check the slug with
                                   `ferrule board list`
The reason lines end in ` hint=<fix>`.
Exit codes: 0 ok, 1 refused, 4 internal error.
See: ferrule guide show cleanup";

/// Shown after `ferrule run status --help`.
const STATUS_AFTER_LONG_HELP: &str = "\
Output, local only (no host call):
  run=<slug> state=<s> lane=<l> branch=<b> issue=<n> pr=<n> pr-state=<draft|ready> gates=<g,…>
                                 The parent card: `-` for a field not set;
                                 gates lists the set ones of gate1, arch,
                                 no-arch and gate2.
  stage=s<n> state=<s> commit=<sha|-> title=<title>
                                 One per stage card, in order; commit is its
                                 last `commit=`.
  worktree=<path> head=<sha> ahead=<n> uncommitted=<n>
                                 Commits since main_branch and files with
                                 uncommitted changes; `worktree=missing
                                 path=<path>` when it is not there.
  gauntlet=<pass|fail> ...       The parent's last gauntlet line, `-` if none.
  ci=<code|->                    The parent's last `ci=`.
  last=<card> <at> <event> ...   The run's last logged line; left out when
                                 the run has logged nothing.
Refusals (exit 1):
  status=refused reason=card hint=check the slug with `ferrule board list`
Exit codes: 0 ok, 1 refused, 4 internal error.
See: ferrule guide show resume";

/// Shown after `ferrule run diff --help`.
const DIFF_AFTER_LONG_HELP: &str = "\
Output: what `git diff <base>...HEAD` prints in the run's worktree (three
dots: the branch's own changes since it left base), whole; with --stat the
file list instead. --specs narrows to docs/specs, --path to any pathspec
(repeat it). --uncommitted prints `git status --short`, then the diff of the
tracked files' uncommitted changes against HEAD: untracked files are listed,
not diffed, and no base is needed. base is main_branch of ferrule.toml unless
--base names another commit.
Refusals (exit 1, nothing else printed):
  diff=refused reason=card hint=check the slug with `ferrule board list`
  diff=refused reason=no-worktree hint=`ferrule gate approve <slug> gate2` adds it
  diff=refused reason=base hint=fetch it, or pass a --base the branch has
Exit codes: 0 ok, 1 refused, 4 internal error.
Examples:
  ferrule run diff <slug> --specs
  ferrule run diff <slug> --base <sha> --stat
See: ferrule guide show reviewer";

/// Shown after `ferrule run log --help`.
const LOG_AFTER_LONG_HELP: &str = "\
Output: what `git log --oneline <base>..HEAD` prints in the run's worktree,
newest first; with --stat each commit's files too. base is main_branch of
ferrule.toml unless --base names another commit.
Refusals (exit 1): the `log=refused reason=card|no-worktree|base hint=…` lines
of `ferrule run diff --help`.
Exit codes: 0 ok, 1 refused, 4 internal error.
See: ferrule guide show reviewer";

/// Shown after `ferrule run plan-diff --help`.
const PLAN_DIFF_AFTER_LONG_HELP: &str = "\
Output: a unified diff from plan.prev.md (plan.md as the last plan review
saw it; gate 2's review round and `stage replan start` save it) to plan.md,
both in the run's directory.
  plan-diff=same                 plan.md has not changed since.
Refusals (exit 1):
  plan-diff=refused reason=no-plan hint=the planner has not written plan.md yet
  plan-diff=refused reason=no-prev hint=no plan review has snapshotted plan.md; read it whole
Exit codes: 0 ok, 1 refused, 4 internal error.
See: ferrule guide show reviewer-plan";

/// Shown after `ferrule run export --help`.
const EXPORT_AFTER_LONG_HELP: &str = "\
Output:
  export=done archive=<id> path=<dir>
      The archive's cards/ and artifacts/ files are written below <dir>.
Refusals:
  export=refused reason=no-archive The run has no archive (or not <id>);
                                   `ferrule run cleanup` creates one and
                                   prints its id. Ends in ` hint=<fix>`.
Exit codes: 0 done, 1 refused, 4 internal error.
See: ferrule guide show cleanup";

#[derive(Debug, Subcommand)]
pub enum RunCommand {
    /// Start a run: its parent card and directory.
    ///
    /// With `--issue`, also the issue's request, its brief and, for a
    /// slice, its epic.
    #[command(after_long_help = INIT_AFTER_LONG_HELP)]
    Init {
        /// The run's slug.
        slug: String,
        /// The issue the user named: a GitHub number or URL, or a Jira key.
        #[arg(long)]
        issue: Option<String>,
    },
    /// Reconcile a run's cards with git and the host before resuming.
    ///
    /// Prints one row per check; run it before an interrupted run goes on.
    #[command(after_long_help = RECONCILE_AFTER_LONG_HELP)]
    Reconcile {
        /// The run's slug.
        slug: String,
        /// Check git only: skip the issue, PR and PR-body rows.
        #[arg(long)]
        local: bool,
    },
    /// Recreate a lost board from the issue, its draft PR and git.
    ///
    /// Also adds the worktree back.
    #[command(after_long_help = REBUILD_AFTER_LONG_HELP)]
    Rebuild {
        /// The tracking issue: a GitHub number or URL, or a Jira key.
        issue: String,
        /// The run's draft PR: a number or URL.
        pr: String,
    },
    /// Clean up a finished run.
    ///
    /// Removes the worktree and the local branch and archives the cards and
    /// files. Refuses while the worktree holds unpushed work or CI is not
    /// green.
    #[command(after_long_help = CLEANUP_AFTER_LONG_HELP)]
    Cleanup {
        /// The run's slug.
        slug: String,
    },
    /// Record and print a run's metrics line.
    ///
    /// Needs the run's CI green. Without a slug, prints every line recorded.
    #[command(after_long_help = METRICS_AFTER_LONG_HELP)]
    Metrics {
        /// The run's slug; without one, every line recorded.
        slug: Option<String>,
        /// Without a slug: only the latest n lines recorded.
        #[arg(long, value_name = "N", conflicts_with = "slug")]
        last: Option<usize>,
    },
    /// Print where a run stands: cards, worktree, gauntlet, CI, last event.
    #[command(after_long_help = STATUS_AFTER_LONG_HELP)]
    Status {
        /// The run's slug.
        slug: String,
    },
    /// Print the run branch's diff against main, in the run's worktree.
    #[command(after_long_help = DIFF_AFTER_LONG_HELP)]
    Diff {
        /// The run's slug.
        slug: String,
        /// Only docs/specs: the run's spec as it stands on the branch.
        #[arg(long)]
        specs: bool,
        /// Only files under this pathspec; repeat for several.
        #[arg(long, value_name = "PATH")]
        path: Vec<String>,
        /// Only the files changed and their line counts.
        #[arg(long)]
        stat: bool,
        /// Diff against this commit instead of main, e.g. the previous
        /// stage's commit.
        #[arg(long, value_name = "REF")]
        base: Option<String>,
        /// The worktree's uncommitted changes instead: `git status --short`,
        /// then the diff against HEAD.
        #[arg(long, conflicts_with = "base")]
        uncommitted: bool,
    },
    /// Print the run branch's commits since main, oldest last.
    #[command(after_long_help = LOG_AFTER_LONG_HELP)]
    Log {
        /// The run's slug.
        slug: String,
        /// Also each commit's changed files.
        #[arg(long)]
        stat: bool,
        /// Commits since this one instead of main.
        #[arg(long, value_name = "REF")]
        base: Option<String>,
    },
    /// Print what changed in plan.md since the last plan review.
    #[command(after_long_help = PLAN_DIFF_AFTER_LONG_HELP)]
    PlanDiff {
        /// The run's slug.
        slug: String,
    },
    /// Write an archived run back out as files.
    #[command(after_long_help = EXPORT_AFTER_LONG_HELP)]
    Export {
        /// The run's slug.
        slug: String,
        /// The archive id `run cleanup` printed [default: the latest].
        #[arg(long)]
        archive: Option<i64>,
        /// Target directory [default: .ferrule/export/<slug>-<archive>].
        #[arg(long)]
        to: Option<PathBuf>,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum GateName {
    /// The brief.
    Gate1,
    /// The architecture.
    Arch,
    /// An epic's slices.
    Epic,
    /// The plan and the tracking issue.
    Gate2,
    /// A stage stop.
    Stage,
    /// A re-plan.
    Replan,
    /// A meaningful spec change.
    Delta,
    /// The branch review.
    Gate3,
    /// The pull request.
    Gate4,
}

#[derive(Debug, Args)]
pub struct ApproveArgs {
    /// The run's slug.
    pub slug: String,
    /// The gate the user approved.
    pub gate: GateName,
    /// The lane the user settled on, recorded on the card.
    #[arg(long, value_parser = ["fast", "full"])]
    pub lane: Option<String>,
    /// gate1: the run is an epic; records `kind: epic` and removes `lane`.
    #[arg(long, conflicts_with = "lane")]
    pub epic: bool,
    /// gate2: `new`, or the number of an issue the run reuses.
    #[arg(long)]
    pub issue: Option<String>,
    /// gate2: the branch [default: feat/<slug>].
    #[arg(long)]
    pub branch: Option<String>,
    /// gate2: stages that stop for approval beyond the plan's, e.g. `s1,s3`.
    #[arg(long)]
    pub stops: Option<String>,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum NextAt {
    /// Planner, plan reviewer, the spec author's issue answer.
    Gate2,
    /// A re-plan's planner and plan reviewer.
    Replan,
    /// The branch review.
    Gate3,
    /// The spec author's PR body.
    Gate4,
    /// `ci=<exit code of gh pr checks>`.
    Ci,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ReplanStep {
    /// Log `replan from=` and snapshot the plan.
    Start,
    /// Swap the stage cards from `from` on for the new plan's.
    Approve,
}

/// `export=refused reason=no-archive`.
fn no_archive() -> Refusal {
    Refusal::new("export", "no-archive")
        .hint("the run has no such archive; `ferrule run cleanup` creates one and prints its id")
}

/// The fix for a reported commit the run refuses.
fn sha_hint(m: core_run::Mismatch, slug: &str, stage: u32) -> String {
    match m {
        core_run::Mismatch::Card => CARD_HINT.to_owned(),
        core_run::Mismatch::Missing => {
            "the worktree has no such commit; ask the implementer for the real sha".to_owned()
        }
        core_run::Mismatch::OffBranch => {
            "the run's branch lacks the commit; it must be committed on that branch".to_owned()
        }
        core_run::Mismatch::CardCommit => {
            format!(
                "commit through `ferrule stage commit {slug}.s{stage}` so the card logs commit="
            )
        }
    }
}

fn layout() -> Layout {
    Layout::new(project::main_checkout())
}

/// What `run diff` and `run log` show.
struct Pick<'a> {
    paths: &'a [String],
    stat: bool,
    base: Option<&'a str>,
}

/// The run's worktree, or the `<verb>` refusal of why there is none.
fn run_worktree(verb: &'static str, slug: &str) -> io::Result<Result<PathBuf, Refusal>> {
    let Some(s) = card_store(slug)? else {
        return Ok(Err(Refusal::card(verb)));
    };
    let wt = core_run::worktree(&s.conn, &layout(), slug, slug).map_err(internal)?;
    if !wt.is_dir() {
        return Ok(Err(Refusal::new(verb, "no-worktree")
            .hint(format!("`ferrule gate approve {slug} gate2` adds it"))));
    }
    Ok(Ok(wt))
}

/// The commit `base` names in `wt` (main by default), or the `<verb>`
/// refusal of why it names none.
fn base_in(
    verb: &'static str,
    wt: &Path,
    base: Option<&str>,
) -> io::Result<Result<String, Refusal>> {
    let base = match base {
        Some(b) => b.to_owned(),
        None => Config::load(&layout().root).map_err(internal)?.main_branch,
    };
    if ferrule_core::git::commit(wt, &base).is_none() {
        return Ok(Err(
            Refusal::new(verb, "base").hint("fetch it, or pass a --base the branch has")
        ));
    }
    Ok(Ok(base))
}

/// Writes what `git <args>` prints in `dir`; a failing git is an internal
/// error, except exit 1 when `differ_ok` (`diff --no-index`: the files differ).
fn git_to(dir: &Path, args: &[&str], differ_ok: bool, out: &mut impl Write) -> io::Result<()> {
    let o = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .stdin(std::process::Stdio::null())
        .output()?;
    if !(o.status.success() || differ_ok && o.status.code() == Some(1)) {
        return Err(internal(GitError {
            args: args.iter().map(|&a| a.to_owned()).collect(),
            code: o.status.code(),
            stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
        }));
    }
    out.write_all(&o.stdout)
}

fn diff(slug: &str, pick: &Pick, uncommitted: bool, out: &mut impl Write) -> io::Result<u8> {
    let found = match run_worktree("diff", slug)? {
        Ok(wt) if uncommitted => Ok((wt, None)),
        Ok(wt) => base_in("diff", &wt, pick.base)?.map(|b| (wt, Some(format!("{b}...HEAD")))),
        Err(r) => Err(r),
    };
    let (wt, range) = match found {
        Ok(found) => found,
        Err(r) => return refuse(out, &r),
    };
    let mut args = vec!["diff", "--no-color", "--no-ext-diff"];
    if pick.stat {
        args.push("--stat");
    }
    if let Some(range) = &range {
        args.push(range);
    } else {
        git_to(&wt, &["status", "--short"], false, out)?;
        args.push("HEAD");
    }
    args.push("--");
    args.extend(pick.paths.iter().map(String::as_str));
    git_to(&wt, &args, false, out)?;
    Ok(exit::OK)
}

fn log(slug: &str, pick: &Pick, out: &mut impl Write) -> io::Result<u8> {
    let found = match run_worktree("log", slug)? {
        Ok(wt) => base_in("log", &wt, pick.base)?.map(|b| (wt, b)),
        Err(r) => Err(r),
    };
    let (wt, base) = match found {
        Ok(found) => found,
        Err(r) => return refuse(out, &r),
    };
    let range = format!("{base}..HEAD");
    let mut args = vec!["log", "--no-color", "--oneline"];
    if pick.stat {
        args.push("--stat");
    }
    args.push(&range);
    git_to(&wt, &args, false, out)?;
    Ok(exit::OK)
}

fn status(slug: &str, out: &mut impl Write) -> io::Result<u8> {
    let layout = layout();
    let config = Config::load(&layout.root).map_err(internal)?;
    let Some(s) = card_store(slug)? else {
        return refuse(out, &Refusal::card("status"));
    };
    let c = &s.conn;
    let last_value = |name: Name| -> io::Result<String> {
        Ok(board::last_value(c, slug, name)
            .map_err(internal)?
            .unwrap_or_else(|| "-".to_owned()))
    };
    let field = |card: &str, key: &str| -> io::Result<String> {
        Ok(board::field(c, card, key)
            .map_err(internal)?
            .unwrap_or_else(|| "-".to_owned()))
    };
    let state = board::state(c, slug)
        .map_err(internal)?
        .map_or("-", |st| st.as_str());
    let mut gates = Vec::new();
    for g in ["gate1", "arch", "no-arch", "gate2"] {
        if board::field(c, slug, g).map_err(internal)?.is_some() {
            gates.push(g);
        }
    }
    writeln!(
        out,
        "run={slug} state={state} lane={} branch={} issue={} pr={} pr-state={} gates={}",
        field(slug, "lane")?,
        field(slug, "branch")?,
        field(slug, "issue")?,
        field(slug, "pr")?,
        last_value(Name::Pr)?,
        if gates.is_empty() {
            "-".to_owned()
        } else {
            gates.join(",")
        }
    )?;
    for (n, id, st) in &board::stages(c, slug).map_err(internal)? {
        let commit = board::last_value(c, id, Name::Commit).map_err(internal)?;
        writeln!(
            out,
            "stage=s{n} state={} commit={} title={}",
            st.as_str(),
            commit.as_deref().unwrap_or("-"),
            field(id, "title")?
        )?;
    }
    let wt = core_run::worktree(c, &layout, slug, slug).map_err(internal)?;
    let shown = wt
        .strip_prefix(&layout.root)
        .unwrap_or(&wt)
        .display()
        .to_string();
    if wt.is_dir() {
        let git =
            |args: &[&str]| ferrule_core::git::out(&wt, args).unwrap_or_else(|| "-".to_owned());
        let range = format!("{}..HEAD", config.main_branch);
        let dirty = ferrule_core::git::out(&wt, &["status", "--porcelain"])
            .map_or(0, |o| o.lines().count());
        writeln!(
            out,
            "worktree={shown} head={} ahead={} uncommitted={dirty}",
            git(&["rev-parse", "--short", "HEAD"]),
            git(&["rev-list", "--count", &range]),
        )?;
    } else {
        writeln!(out, "worktree=missing path={shown}")?;
    }
    let gauntlet = board::log_lines(c, slug, Some("gauntlet=")).map_err(internal)?;
    let gauntlet = gauntlet
        .last()
        .and_then(|(_, body)| body.strip_prefix("gauntlet="));
    writeln!(out, "gauntlet={}", gauntlet.unwrap_or("-"))?;
    writeln!(out, "ci={}", last_value(Name::Ci)?)?;
    if let Some((card, at, body)) = board::run_log_lines(c, slug, None).map_err(internal)?.pop() {
        writeln!(out, "last={card} {at} {body}")?;
    }
    Ok(exit::OK)
}

fn plan_diff(slug: &str, out: &mut impl Write) -> io::Result<u8> {
    let dir = layout().run_dir(slug);
    if !dir.join("plan.md").is_file() {
        let r =
            Refusal::new("plan-diff", "no-plan").hint("the planner has not written plan.md yet");
        return refuse(out, &r);
    }
    if !dir.join("plan.prev.md").is_file() {
        let r = Refusal::new("plan-diff", "no-prev")
            .hint("no plan review has snapshotted plan.md; read it whole");
        return refuse(out, &r);
    }
    let mut text = Vec::new();
    git_to(
        &dir,
        &[
            "diff",
            "--no-index",
            "--no-color",
            "--no-ext-diff",
            "--",
            "plan.prev.md",
            "plan.md",
        ],
        true,
        &mut text,
    )?;
    if text.is_empty() {
        writeln!(out, "plan-diff=same")?;
    } else {
        out.write_all(&text)?;
    }
    Ok(exit::OK)
}

pub fn run(command: &RunCommand, out: &mut impl Write) -> io::Result<u8> {
    match command {
        RunCommand::Init { slug, issue } => init(slug, issue.as_deref(), out),
        RunCommand::Reconcile { slug, local } => reconcile(slug, *local, out),
        RunCommand::Rebuild { issue, pr } => rebuild(issue, pr, out),
        RunCommand::Cleanup { slug } => cleanup(slug, out),
        RunCommand::Export { slug, archive, to } => export(slug, *archive, to.as_deref(), out),
        RunCommand::Metrics { slug, last } => metrics(slug.as_deref(), *last, out),
        RunCommand::Diff {
            slug,
            specs,
            path,
            stat,
            base,
            uncommitted,
        } => {
            let mut paths = path.clone();
            if *specs {
                paths.push("docs/specs".to_owned());
            }
            let pick = Pick {
                paths: &paths,
                stat: *stat,
                base: base.as_deref(),
            };
            diff(slug, &pick, *uncommitted, out)
        }
        RunCommand::Log { slug, stat, base } => {
            let pick = Pick {
                paths: &[],
                stat: *stat,
                base: base.as_deref(),
            };
            log(slug, &pick, out)
        }
        RunCommand::PlanDiff { slug } => plan_diff(slug, out),
        RunCommand::Status { slug } => status(slug, out),
    }
}

pub(super) fn sha_check(slug: &str, stage: u32, sha: &str, out: &mut impl Write) -> io::Result<u8> {
    let verdict = match store(false)? {
        Some(s) => core_run::sha_check(&s.conn, &layout(), slug, &format!("s{stage}"), sha)
            .map_err(internal)?,
        None => Err(core_run::Mismatch::Card),
    };
    match verdict {
        Ok(()) => {
            writeln!(out, "sha=ok")?;
            Ok(exit::OK)
        }
        Err(m) => {
            let hint = sha_hint(m, slug, stage);
            writeln!(out, "sha=mismatch reason={} hint={hint}", m.as_str())?;
            Ok(exit::REFUSED)
        }
    }
}

fn reconcile(slug: &str, local: bool, out: &mut impl Write) -> io::Result<u8> {
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
        let now = board::now();
        let run = publish::Run {
            conn: &mut s.conn,
            layout: &layout,
            config: &config,
            root: &layout.root,
            slug,
            now: &now,
        };
        match run.reconcile_remote() {
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

pub(super) fn stage_stop(
    slug: &str,
    stage: u32,
    threads: u32,
    out: &mut impl Write,
) -> io::Result<u8> {
    let layout = layout();
    let config = Config::load(&layout.root).map_err(internal)?;
    let Some(mut s) = store(false)? else {
        return refuse(out, &Refusal::card("stage-stop"));
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
        Err(Error::Card) => return refuse(out, &Refusal::card("stage-stop")),
        Err(Error::Sql(e)) => return Err(internal(e)),
    }
    Ok(exit::OK)
}

fn metrics(slug: Option<&str>, last: Option<usize>, out: &mut impl Write) -> io::Result<u8> {
    let Some(s) = store(false)? else {
        if let Some(slug) = slug {
            eprintln!("metrics: no card for {slug}");
            return Ok(exit::REFUSED);
        }
        return Ok(exit::OK);
    };
    let Some(slug) = slug else {
        let mut lines = archive::metrics_log(&s.conn).map_err(internal)?;
        if let Some(n) = last {
            lines.drain(..lines.len().saturating_sub(n));
        }
        for line in lines {
            writeln!(out, "{line}")?;
        }
        return Ok(exit::OK);
    };
    match archive::metrics(&s.conn, slug, &board::now()) {
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

fn cleanup(slug: &str, out: &mut impl Write) -> io::Result<u8> {
    let layout = layout();
    let Some(mut s) = store(true)? else {
        unreachable!("store(true) creates the database");
    };
    let refusal = match archive::cleanup(&mut s.conn, &layout, slug, &board::now()) {
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

fn export(
    slug: &str,
    id: Option<i64>,
    to: Option<&std::path::Path>,
    out: &mut impl Write,
) -> io::Result<u8> {
    let layout = layout();
    let Some(s) = store(false)? else {
        return refuse(out, &no_archive());
    };
    let id = match id {
        Some(id) => Some(id),
        None => archive::archives(&s.conn, slug)
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
        std::path::Path::to_path_buf,
    );
    match archive::export(&s.conn, slug, Some(id), &dest) {
        Ok(Some((id, path))) => {
            writeln!(out, "export=done archive={id} path={}", path.display())?;
            Ok(exit::OK)
        }
        Ok(None) => refuse(out, &no_archive()),
        Err(ArchiveError::Io(e)) => Err(e),
        Err(e) => Err(internal(e)),
    }
}

pub(super) fn gauntlet(slug: &str, force: bool, out: &mut impl Write) -> io::Result<u8> {
    let layout = layout();
    let config = Config::load(&layout.root).map_err(internal)?;
    let Some(s) = store(true)? else {
        unreachable!("store(true) creates the database");
    };
    let wt = core_run::worktree(&s.conn, &layout, slug, slug).map_err(internal)?;
    let log = layout.run_dir(slug).join("gauntlet.log");
    let outcome = gauntlet::run(
        &s.conn,
        slug,
        &wt,
        &log,
        &config.gauntlet.step,
        force,
        &board::now(),
    )
    .map_err(internal)?;
    writeln!(out, "{outcome}")?;
    Ok(if matches!(outcome, Outcome::Pass { .. }) {
        exit::OK
    } else {
        exit::REFUSED
    })
}

pub(super) fn next_step(
    slug: &str,
    at: NextAt,
    status: &[String],
    out: &mut impl Write,
) -> io::Result<u8> {
    let at = match at {
        NextAt::Gate2 => At::Gate2,
        NextAt::Replan => At::Replan,
        NextAt::Gate3 => At::Gate3,
        NextAt::Gate4 => At::Gate4,
        NextAt::Ci => At::Ci,
    };
    let status = Status::parse(status);
    let Some(mut s) = store(false)? else {
        return refuse(out, &Refusal::card("run-next"));
    };
    match next::next(&mut s.conn, &layout(), slug, at, &status, &board::now()) {
        Ok(lines) => {
            for l in lines {
                writeln!(out, "{l}")?;
            }
            Ok(exit::OK)
        }
        Err(next::Error::Usage) => {
            eprintln!(
                "usage: ferrule gate next <slug> {} status=<word>…",
                at.as_str()
            );
            Ok(exit::USAGE)
        }
        Err(e) => match e.refusal(at) {
            Some(r) => refuse(out, &r),
            None => Err(internal(e)),
        },
    }
}

/// The remote host behind `run rebuild`.
struct Remote {
    config: Config,
    pr: String,
    issue: String,
    root: std::path::PathBuf,
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

fn rebuild(issue: &str, pr: &str, out: &mut impl Write) -> io::Result<u8> {
    let layout = Layout::new(project::main_checkout());
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

fn gate_failed(
    e: gate::Error,
    verb: &'static str,
    slug: &str,
    out: &mut impl Write,
) -> io::Result<u8> {
    match e {
        gate::Error::Refused(r) => refuse(out, &r.refusal(verb, slug)),
        gate::Error::Publish(e) => super::issue::post_failed(e, verb, out),
        gate::Error::Remote(e) => {
            eprintln!("{verb}: {e}");
            Ok(exit::REFUSED)
        }
        gate::Error::Board(e) => Err(internal(e)),
        gate::Error::Io(e) => Err(e),
        gate::Error::Sql(e) => Err(internal(e)),
    }
}

fn init(slug: &str, issue: Option<&str>, out: &mut impl Write) -> io::Result<u8> {
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
        Err(e) => gate_failed(e, "init", slug, out),
    }
}

pub(super) fn approve(args: &ApproveArgs, out: &mut impl Write) -> io::Result<u8> {
    let g = match args.gate {
        GateName::Gate1 => Gate::Gate1,
        GateName::Arch => Gate::Arch,
        GateName::Epic => Gate::Epic,
        GateName::Gate2 => Gate::Gate2,
        GateName::Stage => Gate::Stage,
        GateName::Replan => Gate::Replan,
        GateName::Delta => Gate::Delta,
        GateName::Gate3 => Gate::Gate3,
        GateName::Gate4 => Gate::Gate4,
    };
    let gate_name = g.as_str();
    let slug = args.slug.as_str();
    let mut pairs = Vec::new();
    let mut pair = |k: &str, v: Option<&String>| {
        if let Some(v) = v {
            pairs.push((k.to_owned(), v.clone()));
        }
    };
    pair("lane", args.lane.as_ref());
    pair("issue", args.issue.as_ref());
    pair("branch", args.branch.as_ref());
    pair("stops", args.stops.as_ref());
    if args.epic {
        pairs.push(("kind".to_owned(), "epic".to_owned()));
    }
    let layout = layout();
    let config = Config::load(&layout.root).map_err(internal)?;
    let Some(mut s) = store(false)? else {
        return refuse(out, &Refusal::card("approve"));
    };
    match gate::approve(
        &mut s.conn,
        &layout,
        &config,
        g,
        slug,
        &pairs,
        &board::now(),
    ) {
        Ok(a) => {
            write!(out, "approve=done gate={gate_name}")?;
            if let Some(i) = a.issue {
                write!(out, " issue={i}")?;
            }
            if let Some(w) = &a.worktree {
                write!(out, " worktree={w}")?;
            }
            if let Some(n) = a.stages {
                write!(out, " stages={n}")?;
            }
            if a.recorded {
                write!(out, " action=recorded")?;
            }
            if !a.record.is_empty() {
                write!(out, " {}", a.record)?;
            }
            writeln!(out)?;
            Ok(exit::OK)
        }
        Err(e) => gate_failed(e, "approve", slug, out),
    }
}

pub(super) fn replan(
    slug: &str,
    step: ReplanStep,
    stops: Option<&str>,
    out: &mut impl Write,
) -> io::Result<u8> {
    let layout = layout();
    let Some(mut s) = store(false)? else {
        return refuse(out, &Refusal::card("replan"));
    };
    let now = board::now();
    let result = match step {
        ReplanStep::Start => gate::replan_start(&mut s.conn, &layout, slug, &now)
            .map(|from| format!("replan=started from=s{from}")),
        ReplanStep::Approve => {
            let stops: Vec<u32> = stops
                .unwrap_or("")
                .split(',')
                .filter_map(|x| x.trim().strip_prefix('s')?.parse().ok())
                .collect();
            gate::replan_approve(&mut s.conn, &layout, slug, &stops, &now)
                .map(|(from, n)| format!("replan=approved from=s{from} stages={n}"))
        }
    };
    match result {
        Ok(line) => {
            writeln!(out, "{line}")?;
            if matches!(step, ReplanStep::Approve) {
                super::board::sync_draft(&mut s, slug, out)?;
            }
            Ok(exit::OK)
        }
        Err(e) => gate_failed(e, "replan", slug, out),
    }
}

pub(super) fn ci(slug: &str, out: &mut impl Write) -> io::Result<u8> {
    let layout = layout();
    let config = Config::load(&layout.root).map_err(internal)?;
    let Some(mut s) = store(false)? else {
        return refuse(out, &Refusal::card("ci"));
    };
    let now = board::now();
    let waited = publish::Run {
        conn: &mut s.conn,
        layout: &layout,
        config: &config,
        root: &layout.root,
        slug,
        now: &now,
    }
    .wait_ci();
    let (code, failed) = match waited {
        Ok(r) => r,
        Err(publish::Error::NoPr) => {
            let r = Refusal::new("ci", "no-pr").hint(format!(
                "no PR is recorded for this run; open it with `ferrule pr open {slug}`"
            ));
            return refuse(out, &r);
        }
        Err(publish::Error::Remote(ferrule_core::remote::Error::NoChecks(_))) => {
            let r = Refusal::new("ci", "no-checks").hint(format!(
                "the PR reports no checks yet; rerun `ferrule ci wait {slug}` once CI has started"
            ));
            return refuse(out, &r);
        }
        Err(e) => return super::issue::post_failed(e, "ci", out),
    };
    write!(out, "ci={code}")?;
    if !failed.is_empty() {
        write!(out, " failed={}", failed.join(","))?;
    }
    writeln!(out)?;
    if code != 0 {
        let branch = board::field(&s.conn, slug, "branch")
            .map_err(internal)?
            .unwrap_or_default();
        if let Ok(lines) = ferrule_core::remote::failed(
            config.backend.ci,
            &branch,
            ferrule_core::remote::Query::default(),
            &layout.root,
        ) {
            for l in lines {
                writeln!(out, "{l}")?;
            }
        }
    }
    let status = Status::parse(&[format!("ci={code}")]);
    match next::next(&mut s.conn, &layout, slug, At::Ci, &status, &now) {
        Ok(lines) => {
            for l in lines {
                writeln!(out, "{l}")?;
            }
            Ok(exit::OK)
        }
        Err(e) => Err(internal(e)),
    }
}
