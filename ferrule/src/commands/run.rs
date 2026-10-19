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

use super::card::{sql, store};
use super::exit;
use crate::cli::stage_id;

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
See: ferrule guide gate1";

/// Shown after `ferrule run approve --help`.
const APPROVE_AFTER_LONG_HELP: &str = "\
Output:
  approve=done gate=<gate> [issue=<n>] [worktree=<path>] [stages=<n>] [<record>]
      The approval is recorded. `<record>` is what the last stop showed:
      `spec-rev=<n>`, `arch-rev=<n>` and `decisions-seen=<lines>`, also logged
      on the card. `issue=`, `worktree=` and `stages=` appear for gate2 only.
Refusals:
  approve=refused reason=card      No such run; check the slug with
                                   `ferrule card board`
  approve=refused reason=no-plan   gate2: plan.md is missing from the run dir
  approve=refused reason=no-stages gate2: plan.md has no `## Shared` stage table
  approve=refused reason=usage     gate2: --issue is neither `new` nor a number
  approve=refused reason=worktree  gate2: `git worktree add` failed
  approve=refused reason=move      gate2: the parent card cannot move to
                                   inprogress
  post=refused reason=no-issue|no-branch|card
                                   gate2: filing the issue failed; an issue body
                                   that fails its checks prints as
                                   `ferrule issue file` does (a host error goes
                                   to stderr, exit 1)
The approve=refused lines end in ` hint=<fix>`.
Exit codes: 0 done, 1 refused, 2 usage.
See: ferrule guide approval";

/// Shown after `ferrule run gauntlet --help`.
const GAUNTLET_AFTER_LONG_HELP: &str = "\
Output (the steps' own output goes to .ferrule/runs/<slug>/gauntlet.log):
  gauntlet=pass cov=<pct> sha=<sha> [cached]
      Every step passed; `cov` is the line coverage of the step marked
      `coverage = \"llvm-cov\"`, `?` when none is. `cached`: this exact tree
      passed before and nothing ran; --force runs it anyway. Not logged on
      the card: `ferrule pr push` logs `gauntlet=pass`.
Failures:
  gauntlet=fail step=<name> sha=<sha> log=<path>
      The step exited non-zero; read the log, fix, rerun.
  gauntlet=fail step=<name> sha=<sha> reason=timeout log=<path>
      The step ran past its timeout and was killed.
  gauntlet=fail step=cd reason=no-worktree
      The run has no worktree; `ferrule run approve <slug> gate2` adds it.
  gauntlet=fail step=config reason=no-steps
      ferrule.toml has no [[gauntlet.step]]; add the project's checks.
The no-worktree and no-steps lines end in ` hint=<fix>`.
Exit codes: 0 pass, 1 fail.
See: ferrule guide verify";

/// Shown after `ferrule run sha-check --help`.
const SHA_CHECK_AFTER_LONG_HELP: &str = "\
Output:
  sha=ok                           The commit is in the worktree, on the run's
                                   branch, and the card's last commit=
Refusals:
  sha=mismatch reason=card         No such run or stage; check the slug and
                                   stage with `ferrule card board`
  sha=mismatch reason=missing      The worktree has no such commit; ask the
                                   implementer for the real sha
  sha=mismatch reason=off-branch   The run's branch lacks the commit
  sha=mismatch reason=card-commit  The stage card's last commit= is another
                                   commit; commit with `ferrule card commit`
Refusals end in ` hint=<fix>`.
Exit codes: 0 ok, 1 mismatch, 2 usage.
See: ferrule guide stages";

/// Shown after `ferrule run stage-stop --help`.
const STAGE_STOP_AFTER_LONG_HELP: &str = "\
Output (also logged on the stage card; s0 never stops):
  auto                             Continue without a stop; logs `auto=yes`
  stop=<n> reason=<why> [detail=\"...\"]
                                   Stop for the user's approval; logs `stop=<n>`.
                                   The first rule that holds wins:
    stop=1 reason=needs-user       review.verdict.md has a Needs user item
    stop=2 reason=verification     plan.summary.md lists the stage under
                                   `Verification beyond unit tests`; the
                                   detail names it
    stop=3 reason=pr-threads       --threads is above 0; detail=\"<n> threads\"
    stop=5 reason=plan-marked      The stage's card has stop: yes
    stop=6 reason=diff-size        The diff since the previous stage is over
                                   the [thresholds] of ferrule.toml; detail=
                                   \"<lines> lines, <files> files\"
    stop=7 reason=checkpoint       Enough stages auto-continued in a row
Refusals:
  stage-stop=card                  No such stage card; check the slug and
                                   stage with `ferrule card board`
The refusal ends in ` hint=<fix>`.
Exit codes: 0 auto or stop, 1 refused, 2 usage.
See: ferrule guide stage-review";

/// Shown after `ferrule run replan --help`.
const REPLAN_AFTER_LONG_HELP: &str = "\
Output:
  replan=started from=s<n>         Logged on the card, plan.md saved as
                                   plan.prev.md; <n> is the lowest stage
                                   without a commit
  replan=approved from=s<n> stages=<n>
                                   The stage cards from s<n> on are replaced
                                   by the new plan's <n> stages; each
                                   `spec-delta <id> added origin=<o>` line of
                                   the dropped stages is logged again on the
                                   parent, so the ids stay retired
  pr-body=pushed|unchanged         After approve: the draft PR's body synced
  pr-body=refused stage-ref=s<n>   The body names stage s<n> that has no card
  pr-body=failed                   The sync failed (details on stderr)
Refusals:
  replan=refused reason=card       No such run; check the slug with
                                   `ferrule card board`
  replan=refused reason=move       start: the parent cannot move to inprogress
  replan=refused reason=no-replan  approve before start
  replan=refused reason=no-plan    approve: plan.md is missing
  replan=refused reason=no-stages  approve: the plan has no stage from s<n> on
Refusals end in ` hint=<fix>`.
Exit codes: 0 ok, 1 refused, 2 usage.
See: ferrule guide replan";

/// Shown after `ferrule run ci --help`.
const CI_AFTER_LONG_HELP: &str = "\
Output:
  ci=<code> [failed=<jobs>]        The checks' exit code (0 is green) and,
                                   when red, the comma-separated failed jobs
  <error line>...                  When red: the failed run's error lines
  next=, run:, do:, guide: lines   The next step, as `ferrule run next <slug>
                                   ci ci=<code>` prints it; see its help
The output of the wait is saved in .ferrule/runs/<slug>/ci.log.
Refusals:
  ci=refused reason=card           No such run; check the slug with
                                   `ferrule card board`
  post=refused reason=card         The run has no PR recorded; open it with
                                   `ferrule pr open <slug>`
  (stderr: the host's error)       The checks could not be read (exit 1)
The ci=refused line ends in ` hint=<fix>`.
Exit codes: 0 ok, 1 refused.
See: ferrule guide gate4";

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
                                   `ferrule card board`
Exit codes: 0 clean, 1 differ, 2 usage.
See: ferrule guide resume";

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
See: ferrule guide resume";

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
  cleanup=refused reason=no-ci                No ci= logged; `ferrule run ci`
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
See: ferrule guide cleanup";

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
Without a slug: every recorded line, oldest run first (nothing if none).
Refusals:
  metrics=refused reason=no-ci     No ci= logged; run `ferrule run ci <slug>`
  metrics=refused reason=ci-red    The last ci= is not 0 and no later
                                   ci-waived line exists
  (stderr) metrics: no card for <slug>
                                   Exit 1; check the slug with
                                   `ferrule card board`
The reason lines end in ` hint=<fix>`.
Exit codes: 0 ok, 1 refused.
See: ferrule guide cleanup";

/// Shown after `ferrule run export --help`.
const EXPORT_AFTER_LONG_HELP: &str = "\
Output:
  export=done archive=<id> path=<dir>
      The archive's cards/ and artifacts/ files are written below <dir>.
Refusals:
  export=none                      The run has no archive (or not <id>);
                                   `ferrule run cleanup` creates one and
                                   prints its id. Ends in ` hint=<fix>`.
Exit codes: 0 done, 1 refused.
See: ferrule guide cleanup";

/// Shown after `ferrule run next --help`.
const NEXT_AFTER_LONG_HELP: &str = "\
Statuses each position takes:
  gate2    ready, findings, clean, question, arch-gap, split, behavior-change,
           new, reuse (with issue=<n>)
  replan   ready, findings, clean, question, arch-gap, split, behavior-change
  gate3    findings (with count=<n>), clean
  gate4    ready, question
  ci       ci=<exit code of gh pr checks>

Output, in order:
  logged: <event>                  An event logged on the card:
                                   `plan-review=findings count=<n> round=<r>`,
                                   `plan-review=clean round=<r>`,
                                   `review=<n>-open scope=branch`,
                                   `review=<n>-resolved`, `review=clean`,
                                   `replan=pre-authorized` or `ci=<code>`
  next=<word> [reason=<why>]       The one next action:
    relay            question: relay the question file to the user
    arch-gap         spawn the architect, then resume the planner
    split            handle the split request
    behavior-change  back to the spec author
    review           spawn the plan reviewer
    follow-up        spawn the plan reviewer again after findings
    revise           plan review found issues (rounds 1 and 2): resume the
                     planner with the review file
    issue            plan review is done (clean, or findings in round 3): ask
                     the spec author for the tracking issue
    stop             show the stop and wait for the user's approval
    notice           show the notice, then go on as if approved
    fix              gate3: findings are open; ci: CI is red
    stage-stop       gate3 clean with a stage not done: decide its stop
    pr               gate3 clean on every stage: spawn the spec author for
                     the PR body
    done             ci=0: clean up the run
  reason=<why>                     Why `stop` stops, or `pr` has `needs-user`.
                                   gate2 stops for: lane (not fast),
                                   no-arch-stop, needs-user, outgrown (plan
                                   over two stages) and reuse. A re-plan stops
                                   for: not-gate4, needs-user, arch-moved and
                                   outgrown; otherwise it logs
                                   replan=pre-authorized and gives `notice`.
  run: <command>                   A command to run now, slug filled in
  do: <what>                       What to do next (spawn, resume, show)
  guide: ferrule guide <topic> --slug <slug>
                                   The topic to read before acting
Errors:
  run-next=card                    No such run (exit 1); check the slug with
                                   `ferrule card board`
  run-next=unknown at=<pos> status=<word>
                                   The position does not take that status
                                   (exit 2); the list above has the valid ones
  usage: ferrule run next <slug> <pos> status=<word>  (stderr, exit 2)
                                   The status line lacks status= (ci=<code>
                                   at ci)
The run-next lines end in ` hint=<fix>`.
Exit codes: 0 ok, 1 refused, 2 usage.

Example:
  ferrule run next <slug> gate2 status=ready file=plan.md";

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
    /// Record an approved gate.
    ///
    /// `gate2` also files the issue, adds the worktree, moves the parent
    /// card to inprogress and creates the stage cards.
    #[command(after_long_help = APPROVE_AFTER_LONG_HELP)]
    Approve(ApproveArgs),
    /// Log an agent's outcome and print the next step.
    ///
    /// Prints `next=`, then `run:` and `do:` lines and a `guide:` line
    /// naming the topic to read next.
    ///
    /// Reads `status=`, `count=`, `issue=` and `ci=` from the status line and
    /// ignores the rest. Prints `run-next=unknown` (exit 2) for a status the
    /// position does not take.
    #[command(after_long_help = NEXT_AFTER_LONG_HELP)]
    Next {
        /// The run's slug.
        slug: String,
        /// Where the run is: the step whose agent wrote the status line.
        #[arg(value_enum)]
        position: NextAt,
        /// The agent's status line, as one argument or several.
        #[arg(
            required = true,
            allow_hyphen_values = true,
            value_name = "STATUS_LINE"
        )]
        status: Vec<String>,
    },
    /// Run the gauntlet in the run's worktree.
    ///
    /// The steps are `[[gauntlet.step]]` of `ferrule.toml`. Prints one
    /// status line; the output goes to the run's gauntlet.log.
    #[command(after_long_help = GAUNTLET_AFTER_LONG_HELP)]
    Gauntlet {
        /// The run's slug.
        slug: String,
        /// Run even when this tree passed before.
        #[arg(long)]
        force: bool,
    },
    /// Verify a stage's reported commit before anything is pushed.
    ///
    /// The commit exists in the worktree, the run's branch holds it, and
    /// it is the stage card's last `commit=`.
    #[command(after_long_help = SHA_CHECK_AFTER_LONG_HELP)]
    ShaCheck {
        /// The run's slug.
        slug: String,
        /// The stage, `s<n>`.
        #[arg(value_parser = stage_id, value_name = "STAGE")]
        stage: u32,
        /// The commit the implementer reported.
        sha: String,
    },
    /// Decide whether a green full-lane stage stops for approval.
    ///
    /// Logs the outcome on the stage card.
    #[command(after_long_help = STAGE_STOP_AFTER_LONG_HELP)]
    StageStop {
        /// The run's slug.
        slug: String,
        /// The stage, `s<n>`.
        #[arg(value_parser = stage_id, value_name = "STAGE")]
        stage: u32,
        /// Unresolved PR review threads.
        #[arg(long, default_value_t = 0)]
        threads: u32,
    },
    /// Start or approve a re-plan.
    #[command(after_long_help = REPLAN_AFTER_LONG_HELP)]
    Replan {
        /// The run's slug.
        slug: String,
        /// Which half of the re-plan.
        step: ReplanStep,
        /// With `approve`: stages that stop for approval beyond the plan's,
        /// e.g. `s1,s3`.
        #[arg(long)]
        stops: Option<String>,
    },
    /// Wait for the PR's checks and print the next step.
    ///
    /// Logs `ci=`; on red, also prints the failed run's error lines.
    #[command(after_long_help = CI_AFTER_LONG_HELP)]
    Ci {
        /// The run's slug.
        slug: String,
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

const CARD_HINT: &str = "check the slug with `ferrule card board`";
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
            format!("the parent card cannot move to inprogress; see `ferrule card show {slug}`")
        }
        "no-replan" => format!("start it first: ferrule run replan {slug} start"),
        "detached-head" => "check the run's branch out in its worktree".to_owned(),
        "uncommitted-changes" => {
            "commit them with `ferrule card commit`, or ask the user, then push".to_owned()
        }
        "no-upstream" | "unpushed-commits" => format!("push the branch: ferrule pr push {slug}"),
        "worktree-remove-failed" => {
            "the shell guard refuses `git worktree remove`; ask the user to remove it, then rerun"
                .to_owned()
        }
        "branch-delete-failed" => {
            "delete the branch by hand with `git branch -D`, then rerun".to_owned()
        }
        "no-ci" => format!("record CI first: ferrule run ci {slug}"),
        "ci-red" => format!(
            "fix CI and rerun `ferrule run ci {slug}`; on the user's word only: ferrule card log {slug} ci-waived -- <why>"
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
            format!("commit through `ferrule card commit {slug}.s{stage}` so the card logs commit=")
        }
    }
}

fn layout() -> Layout {
    Layout::new(project::main_checkout())
}

pub fn run(command: &RunCommand, out: &mut impl Write) -> io::Result<u8> {
    match command {
        RunCommand::ShaCheck { slug, stage, sha } => sha_check(slug, *stage, sha, out),
        RunCommand::Reconcile { slug, local } => reconcile(slug, *local, out),
        RunCommand::Init { slug, issue } => init(slug, issue.as_deref(), out),
        RunCommand::Approve(args) => approve(args, out),
        RunCommand::Replan { slug, step, stops } => replan(slug, *step, stops.as_deref(), out),
        RunCommand::Ci { slug } => ci(slug, out),
        RunCommand::Gauntlet { slug, force } => gauntlet(slug, *force, out),
        RunCommand::Next {
            slug,
            position,
            status,
        } => next_step(slug, *position, status, out),
        RunCommand::Metrics { slug } => metrics(slug.as_deref(), out),
        RunCommand::Cleanup { slug } => cleanup(slug, out),
        RunCommand::Rebuild { issue, pr } => rebuild(issue, pr, out),
        RunCommand::Export { slug, archive, to } => export(slug, *archive, to.as_deref(), out),
        RunCommand::StageStop {
            slug,
            stage,
            threads,
        } => stage_stop(slug, *stage, *threads, out),
    }
}

fn sha_check(slug: &str, stage: u32, sha: &str, out: &mut impl Write) -> io::Result<u8> {
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

fn stage_stop(slug: &str, stage: u32, threads: u32, out: &mut impl Write) -> io::Result<u8> {
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

fn metrics(slug: Option<&str>, out: &mut impl Write) -> io::Result<u8> {
    let Some(s) = store(false)? else {
        if let Some(slug) = slug {
            eprintln!("metrics: no card for {slug}");
            return Ok(exit::REFUSED);
        }
        return Ok(exit::OK);
    };
    let Some(slug) = slug else {
        for line in archive::metrics_log(&s.conn).map_err(sql)? {
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

fn gauntlet(slug: &str, force: bool, out: &mut impl Write) -> io::Result<u8> {
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

fn next_step(slug: &str, at: NextAt, status: &[String], out: &mut impl Write) -> io::Result<u8> {
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
                "usage: ferrule run next <slug> {} status=<word>…",
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
                "run-next=unknown at={} status={word} hint=`ferrule run next --help` lists the statuses each position takes",
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

fn approve(args: &ApproveArgs, out: &mut impl Write) -> io::Result<u8> {
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

fn replan(
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
                super::card::sync_draft(&mut s, slug, out)?;
            }
            Ok(exit::OK)
        }
        Err(e) => gate_failed(e, "replan", slug, out),
    }
}

fn ci(slug: &str, out: &mut impl Write) -> io::Result<u8> {
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
