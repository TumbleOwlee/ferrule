use std::io::{self, Write};
use std::path::PathBuf;

use clap::{Args, Subcommand, ValueEnum};
use ferrule_core::card::board;
use ferrule_core::config::Config;
use ferrule_core::pr::publish;
use ferrule_core::project::{self, Layout};
use ferrule_core::remote::{self, Item};
use ferrule_core::run::archive::{self, Error as ArchiveError};
use ferrule_core::run::gate::{self, Gate};
use ferrule_core::run::gauntlet::{self, Outcome};
use ferrule_core::run::next::{self, At, Status};
use ferrule_core::run::rebuild;
use ferrule_core::run::{self as core_run, Error};
use ferrule_core::store::DIR;

use super::board::{sql, store};
use super::exit;

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
                                   resume it with `ferrule run reconcile <slug>`
Errors (stderr):
  init: <host error>               The issue could not be read (exit 1)
Refusals end in ` hint=<fix>`.
Exit codes: 0 done, 1 refused, 2 usage.
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
Exit codes: 0 clean, 1 differ, 2 usage.
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
  rebuild=refused reason=worktree      `git worktree add` failed
Refusals end in ` hint=<fix>`.
Exit codes: 0 done, 1 refused, 2 usage.
See: ferrule guide show resume";

/// Shown after `ferrule run cleanup --help`.
const CLEANUP_AFTER_LONG_HELP: &str = "\
Output:
  cleanup=done archive=<id>        The metrics line is recorded, the worktree
                                   and local branch are removed, and the cards
                                   and run files are archived as <id>
                                   (`ferrule run export` writes it back)
  cleanup=done archive=none        Nothing was left to archive
Refusals (nothing is removed):
  cleanup=refused reason=card                 No such run
  cleanup=refused reason=detached-head        The worktree is on no branch
  cleanup=refused reason=uncommitted-changes  The worktree has changes; commit
                                              them, or ask the user, then push
  cleanup=refused reason=no-upstream          The branch was never pushed
  cleanup=refused reason=unpushed-commits     Push with `ferrule pr push <slug>`
  cleanup=refused reason=no-ci                No ci= logged; `ferrule ci wait`
  cleanup=refused reason=ci-red               The last ci= is not 0 and no later
                                              ci-waived line exists
  cleanup=refused reason=worktree-remove-failed
                                              git could not remove the worktree;
                                              ask the user (the shell guard
                                              refuses `git worktree remove`)
  cleanup=refused reason=branch-delete-failed The local branch could not be
                                              deleted; `git branch -D` it
Refusals end in ` hint=<fix>`.
Exit codes: 0 done, 1 refused.
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
Exit codes: 0 ok, 1 refused.
See: ferrule guide show cleanup";

/// Shown after `ferrule run export --help`.
const DIFF_AFTER_LONG_HELP: &str = "\
Output: what `git diff <base>...HEAD` prints in the run's worktree (three
dots: the branch's own changes since it left base), whole; with --stat the
file list instead. --specs narrows to docs/specs, --path to any pathspec
(repeat it). --uncommitted prints `git status --short`, then the diff of the
uncommitted work against HEAD. base is main_branch of ferrule.toml unless
--base names another commit.
Refusals (exit 1, nothing else printed):
  diff=refused reason=card hint=check the slug with `ferrule board list`
  diff=refused reason=no-worktree hint=`ferrule gate approve <slug> gate2` adds it
  diff=refused reason=base hint=fetch it, or pass a --base the branch has
Exit codes: 0 ok, 1 refused.
Examples:
  ferrule run diff <slug> --specs
  ferrule run diff <slug> --base <sha> --stat
See: ferrule guide show reviewer";

const LOG_AFTER_LONG_HELP: &str = "\
Output: what `git log --oneline <base>..HEAD` prints in the run's worktree,
newest first; with --stat each commit's files too. base is main_branch of
ferrule.toml unless --base names another commit.
Refusals (exit 1): the `log=refused reason=card|no-worktree|base hint=…` lines
of `ferrule run diff --help`.
Exit codes: 0 ok, 1 refused.
See: ferrule guide show reviewer";

const PLAN_DIFF_AFTER_LONG_HELP: &str = "\
Output: a unified diff from plan.prev.md (plan.md as the last plan review
saw it; gate 2's review round and `stage replan start` save it) to plan.md,
both in the run's directory.
  plan-diff=same                 plan.md has not changed since.
Refusals (exit 1):
  plan-diff=refused reason=no-plan hint=the planner has not written plan.md yet
  plan-diff=refused reason=no-prev hint=no plan review has snapshotted plan.md; read it whole
Exit codes: 0 ok, 1 refused.
See: ferrule guide show reviewer-plan";

const EXPORT_AFTER_LONG_HELP: &str = "\
Output:
  export=done archive=<id> path=<dir>
      The archive's cards/ and artifacts/ files are written below <dir>.
Refusals:
  export=none                      The run has no archive (or not <id>);
                                   `ferrule run cleanup` creates one and
                                   prints its id. Ends in ` hint=<fix>`.
Exit codes: 0 done, 1 refused.
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
    /// gate1: the run is an epic.
    #[arg(long)]
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

const CARD_HINT: &str = "check the slug with `ferrule board list`";
const EXPORT_HINT: &str =
    "the run has no such archive; `ferrule run cleanup` creates one and prints its id";

/// ` hint=<fix>` for a refusal `reason`, or nothing when it has no fix to name.
fn hint(reason: &str, slug: &str) -> String {
    let fix = match reason {
        "card" => CARD_HINT.to_owned(),
        "exists" => format!("the run exists; resume it with `ferrule run reconcile {slug}`"),
        "no-plan" => format!("write .ferrule/runs/{slug}/plan.md first"),
        "no-stages" => "add the `## Shared` stage table to plan.md".to_owned(),
        "usage" => "--issue takes `new` or an issue number".to_owned(),
        "worktree" => {
            "git could not add the worktree; check `git worktree list` and --branch".to_owned()
        }
        "move" => {
            format!("the parent card cannot move to inprogress; see `ferrule board show {slug}`")
        }
        "no-replan" => format!("start it first: ferrule stage replan {slug} start"),
        "detached-head" => "check the run's branch out in its worktree".to_owned(),
        "uncommitted-changes" => {
            "commit them with `ferrule stage commit`, or ask the user, then push".to_owned()
        }
        "no-upstream" | "unpushed-commits" => format!("push the branch: ferrule pr push {slug}"),
        "worktree-remove-failed" => {
            "the shell guard refuses `git worktree remove`; ask the user to remove it, then rerun"
                .to_owned()
        }
        "branch-delete-failed" => {
            "delete the branch by hand with `git branch -D`, then rerun".to_owned()
        }
        "no-ci" => format!("record CI first: ferrule ci wait {slug}"),
        "ci-red" => format!(
            "fix CI and rerun `ferrule ci wait {slug}`; on the user's word only: ferrule board log {slug} ci-waived -- <why>"
        ),
        _ => return String::new(),
    };
    format!(" hint={fix}")
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

/// The run's worktree and the commit `base` names (main by default), or
/// the `reason=… hint=…` of why there is none.
fn branch(slug: &str, base: Option<&str>) -> io::Result<Result<(PathBuf, String), String>> {
    let layout = layout();
    let config = Config::load(&layout.root).map_err(sql)?;
    let s = match store(false)? {
        Some(s) if board::state(&s.conn, slug).map_err(sql)?.is_some() => s,
        _ => {
            return Ok(Err(
                "reason=card hint=check the slug with `ferrule board list`".to_owned(),
            ));
        }
    };
    let wt = board::field(&s.conn, slug, "worktree")
        .map_err(sql)?
        .map_or_else(|| layout.worktree(slug), |w| layout.resolve(&w));
    if !wt.is_dir() {
        return Ok(Err(format!(
            "reason=no-worktree hint=`ferrule gate approve {slug} gate2` adds it"
        )));
    }
    let base = base.map_or(config.main_branch, str::to_owned);
    if ferrule_core::git::commit(&wt, &base).is_none() {
        return Ok(Err(
            "reason=base hint=fetch it, or pass a --base the branch has".to_owned(),
        ));
    }
    Ok(Ok((wt, base)))
}

/// Writes what `git <args>` prints in `dir`; a failing git is an error.
fn git_to(dir: &std::path::Path, args: &[&str], out: &mut impl Write) -> io::Result<()> {
    let o = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .stdin(std::process::Stdio::null())
        .output()?;
    // `diff --no-index` exits 1 when the files differ
    if !o.status.success() && o.status.code() != Some(1) {
        return Err(io::Error::other(
            String::from_utf8_lossy(&o.stderr).trim().to_owned(),
        ));
    }
    out.write_all(&o.stdout)
}

fn diff(slug: &str, pick: &Pick, uncommitted: bool, out: &mut impl Write) -> io::Result<u8> {
    let (wt, base) = match branch(slug, pick.base)? {
        Ok(found) => found,
        Err(why) => {
            writeln!(out, "diff=refused {why}")?;
            return Ok(exit::REFUSED);
        }
    };
    let range = format!("{base}...HEAD");
    let mut args = vec!["diff", "--no-color", "--no-ext-diff"];
    if pick.stat {
        args.push("--stat");
    }
    if uncommitted {
        git_to(&wt, &["status", "--short"], out)?;
        args.push("HEAD");
    } else {
        args.push(&range);
    }
    args.push("--");
    args.extend(pick.paths.iter().map(String::as_str));
    git_to(&wt, &args, out)?;
    Ok(exit::OK)
}

fn log(slug: &str, pick: &Pick, out: &mut impl Write) -> io::Result<u8> {
    let (wt, base) = match branch(slug, pick.base)? {
        Ok(found) => found,
        Err(why) => {
            writeln!(out, "log=refused {why}")?;
            return Ok(exit::REFUSED);
        }
    };
    let range = format!("{base}..HEAD");
    let mut args = vec!["log", "--no-color", "--oneline"];
    if pick.stat {
        args.push("--stat");
    }
    args.push(&range);
    git_to(&wt, &args, out)?;
    Ok(exit::OK)
}

fn plan_diff(slug: &str, out: &mut impl Write) -> io::Result<u8> {
    let dir = layout().run_dir(slug);
    if !dir.join("plan.md").is_file() {
        writeln!(
            out,
            "plan-diff=refused reason=no-plan hint=the planner has not written plan.md yet"
        )?;
        return Ok(exit::REFUSED);
    }
    if !dir.join("plan.prev.md").is_file() {
        writeln!(
            out,
            "plan-diff=refused reason=no-prev hint=no plan review has snapshotted plan.md; read it whole"
        )?;
        return Ok(exit::REFUSED);
    }
    let mut text = Vec::new();
    git_to(
        &dir,
        &[
            "diff",
            "--no-index",
            "--no-color",
            "--",
            "plan.prev.md",
            "plan.md",
        ],
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
    }
}

pub(super) fn sha_check(slug: &str, stage: u32, sha: &str, out: &mut impl Write) -> io::Result<u8> {
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
        Err(Error::Sql(e)) => return Err(sql(e)),
    };
    if !local && let Some(s) = store.as_mut() {
        let config = Config::load(&layout.root).map_err(sql)?;
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
            Err(e) => r.differ.push(format!("host unreadable: {e:?}")),
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
    let config = Config::load(&layout.root).map_err(sql)?;
    let Some(mut s) = store(false)? else {
        writeln!(out, "stage-stop=card hint={CARD_HINT}")?;
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
            writeln!(out, "stage-stop=card hint={CARD_HINT}")?;
            return Ok(exit::REFUSED);
        }
        Err(Error::Sql(e)) => return Err(sql(e)),
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
        let mut lines = archive::metrics_log(&s.conn).map_err(sql)?;
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
        Err(ArchiveError::Ci(r)) => {
            writeln!(
                out,
                "metrics=refused reason={}{}",
                r.as_str(),
                hint(r.as_str(), slug)
            )?;
            Ok(exit::REFUSED)
        }
        Err(e) => Err(sql(format!("{e:?}"))),
    }
}

fn cleanup(slug: &str, out: &mut impl Write) -> io::Result<u8> {
    let layout = layout();
    let Some(mut s) = store(true)? else {
        unreachable!("store(true) creates the database");
    };
    let reason = match archive::cleanup(&mut s.conn, &layout, slug, &board::now()) {
        Ok(Some(id)) => {
            writeln!(out, "cleanup=done archive={id}")?;
            return Ok(exit::OK);
        }
        Ok(None) => {
            writeln!(out, "cleanup=done archive=none")?;
            return Ok(exit::OK);
        }
        Err(ArchiveError::Refused(r)) => r,
        Err(ArchiveError::Ci(r)) => r.as_str(),
        Err(ArchiveError::Card) => "card",
        Err(ArchiveError::Io(e)) => return Err(e),
        Err(ArchiveError::Sql(e)) => return Err(sql(e)),
    };
    writeln!(out, "cleanup=refused reason={reason}{}", hint(reason, slug))?;
    Ok(exit::REFUSED)
}

fn export(
    slug: &str,
    id: Option<i64>,
    to: Option<&std::path::Path>,
    out: &mut impl Write,
) -> io::Result<u8> {
    let layout = layout();
    let Some(s) = store(false)? else {
        writeln!(out, "export=none hint={EXPORT_HINT}")?;
        return Ok(exit::REFUSED);
    };
    let id = match id {
        Some(id) => Some(id),
        None => archive::archives(&s.conn, slug)
            .map_err(sql)?
            .last()
            .map(|(id, _)| *id),
    };
    let Some(id) = id else {
        writeln!(out, "export=none hint={EXPORT_HINT}")?;
        return Ok(exit::REFUSED);
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
        Ok(None) => {
            writeln!(out, "export=none hint={EXPORT_HINT}")?;
            Ok(exit::REFUSED)
        }
        Err(ArchiveError::Io(e)) => Err(e),
        Err(e) => Err(sql(format!("{e:?}"))),
    }
}

pub(super) fn gauntlet(slug: &str, force: bool, out: &mut impl Write) -> io::Result<u8> {
    let layout = layout();
    let config = Config::load(&layout.root).map_err(sql)?;
    let Some(s) = store(true)? else {
        unreachable!("store(true) creates the database");
    };
    let wt = board::field(&s.conn, slug, "worktree")
        .map_err(sql)?
        .map_or_else(|| layout.worktree(slug), |w| layout.resolve(&w));
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
    .map_err(sql)?;
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
        writeln!(out, "run-next=card hint={CARD_HINT}")?;
        return Ok(exit::REFUSED);
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
        Err(next::Error::Card) => {
            writeln!(out, "run-next=card hint={CARD_HINT}")?;
            Ok(exit::REFUSED)
        }
        Err(next::Error::Unknown(word)) => {
            writeln!(
                out,
                "run-next=unknown at={} status={word} hint=`ferrule gate next --help` lists the statuses each position takes",
                at.as_str()
            )?;
            Ok(exit::USAGE)
        }
        Err(next::Error::Sql(e)) => Err(sql(e)),
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
    let config = Config::load(&layout.root).map_err(sql)?;
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
        Err(rebuild::Error::Refused(r)) => {
            writeln!(out, "{r}")?;
            Ok(exit::REFUSED)
        }
        Err(rebuild::Error::Sql(e)) => Err(sql(e)),
        Err(rebuild::Error::Board(e)) => Err(sql(format!("{e:?}"))),
        Err(rebuild::Error::Io(e)) => Err(e),
    }
}

fn gate_failed(e: gate::Error, verb: &str, slug: &str, out: &mut impl Write) -> io::Result<u8> {
    match e {
        gate::Error::Refused(reason) => {
            writeln!(out, "{verb}=refused reason={reason}{}", hint(reason, slug))?;
            Ok(exit::REFUSED)
        }
        gate::Error::Publish(e) => super::issue::post_failed(e, out),
        gate::Error::Remote(e) => {
            eprintln!("{verb}: {e}");
            Ok(exit::REFUSED)
        }
        gate::Error::Board(e) => Err(sql(format!("{e:?}"))),
        gate::Error::Io(e) => Err(e),
        gate::Error::Sql(e) => Err(sql(e)),
    }
}

fn init(slug: &str, issue: Option<&str>, out: &mut impl Write) -> io::Result<u8> {
    let layout = layout();
    let config = Config::load(&layout.root).map_err(sql)?;
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
    let (g, gate_name) = match args.gate {
        GateName::Gate1 => (Gate::Gate1, "gate1"),
        GateName::Arch => (Gate::Arch, "arch"),
        GateName::Epic => (Gate::Epic, "epic"),
        GateName::Gate2 => (Gate::Gate2, "gate2"),
        GateName::Stage => (Gate::Other, "stage"),
        GateName::Replan => (Gate::Other, "replan"),
        GateName::Delta => (Gate::Other, "delta"),
        GateName::Gate3 => (Gate::Other, "gate3"),
        GateName::Gate4 => (Gate::Other, "gate4"),
    };
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
    let config = Config::load(&layout.root).map_err(sql)?;
    let Some(mut s) = store(false)? else {
        writeln!(out, "approve=refused reason=card{}", hint("card", slug))?;
        return Ok(exit::REFUSED);
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
        writeln!(out, "replan=refused reason=card{}", hint("card", slug))?;
        return Ok(exit::REFUSED);
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
    let config = Config::load(&layout.root).map_err(sql)?;
    let Some(mut s) = store(false)? else {
        writeln!(out, "ci=refused reason=card{}", hint("card", slug))?;
        return Ok(exit::REFUSED);
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
        Err(e) => return super::issue::post_failed(e, out),
    };
    write!(out, "ci={code}")?;
    if !failed.is_empty() {
        write!(out, " failed={}", failed.join(","))?;
    }
    writeln!(out)?;
    if code != 0 {
        let branch = board::field(&s.conn, slug, "branch")
            .map_err(sql)?
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
        Err(e) => Err(sql(format!("{e:?}"))),
    }
}
