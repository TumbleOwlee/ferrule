//! `ferrule run`: a run's life, from init to cleanup, and what it holds.

mod finish;
mod inspect;
mod start;

use ferrule_core::card::board::Slug;
use std::io::{self, Write};
use std::path::PathBuf;

use clap::{Subcommand, ValueEnum};
use ferrule_core::remote::ItemRef;

/// Shown after `ferrule run init --help`.
const INIT_AFTER_LONG_HELP: &str = "\
Output:
  init=done slug=<slug>            The card and .ferrule/runs/<slug>/ exist
  init=done slug=<slug> kind=slice milestone=<n> slice=<k>
                                   --issue is the <k>th issue of milestone <n>;
                                   the card records kind, milestone and slice
  init=done slug=<slug> kind=slice epic=<n>
                                   Without milestones, --issue named a slice of
                                   epic <n>; the card records kind and epic
With --issue the run directory also holds request.md and issue-spec.md (when
the issue carries them) and, for a slice, epic.md (the milestone's
description, or the epic as it stands); the card records the named issue as
`named-issue`.
A slice whose issue has open blockers is refused until the user approves;
--start-blocked then starts it and the card logs `blocked-start`.
With --request the card records `kind=request` and no slice is detected: no
epic is read and no slice-line.md or epic.md is written.
Refusals:
  init=refused reason=exists       The run already has a card or stage cards;
                                   resume it with `ferrule run reconcile <slug>`
  init=refused reason=blocked issue=<n> blockers=<list> [unreadable=<k>] hint=<fix>
                                   The slice has open blockers; <list> is their
                                   cites joined by `,`, `-` when none shows;
                                   <k> counts open blockers the host does not
                                   show. Ask the user, then re-run with
                                   --start-blocked
Errors (stderr):
  init: <host error>               The issue could not be read (exit 1)
Refusals end in ` hint=<fix>`.
Exit codes: 0 done, 1 refused, 2 usage, 4 internal error.
Dev build: the open that takes a copy of an older shared board prints to stderr
`ferrule=dev-copy copy=<path> shared=<n> schema=<m>`; the exit code is unchanged.
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
    diverged <b> local=<sha> remote=<sha>
                                   The origin's branch holds commits the local
                                   one lacks; a check that cannot complete
                                   adds no row; none after a rebase recorded
                                   since the last push
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
Refusals:
  reconcile=refused reason=card hint=check the slug with `ferrule board list`
                                   No card for the slug
Exit codes: 0 clean, 1 differ or refused, 2 usage, 4 internal error.
Dev build: the open that takes a copy of an older shared board prints to stderr
`ferrule=dev-copy copy=<path> shared=<n> schema=<m>`; the exit code is unchanged.
See: ferrule guide show resume";

/// Shown after `ferrule run live-check --help`.
const LIVE_CHECK_AFTER_LONG_HELP: &str = "\
Opens or closes the run's live check. While it is open, a `gh api` write to the
`live-repo` named at gate 2 is allowed; every other write stays refused.
Output:
  live-check=open repo=<owner/name>
      The check is open for that repository
  live-check=closed
      The check is closed
  live-check=refused reason=no-repo hint=name the repository at gate 2: ferrule
  gate approve <slug> gate2 --live-repo <owner/name>
      The run names no live-repo
  live-check=refused reason=card hint=check the slug with `ferrule board list`
      No such card
Exit codes: 0 done, 1 refused, 2 usage, 4 internal error.
Dev build: the open that takes a copy of an older shared board prints to stderr
`ferrule=dev-copy copy=<path> shared=<n> schema=<m>`; the exit code is unchanged.
See: ferrule guide show gate2";

/// What `run live-check` does.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum LiveAction {
    Start,
    End,
}

/// Shown after `ferrule run rebase --help`.
const REBASE_AFTER_LONG_HELP: &str = "\
Rebases the run's branch onto the base (default `origin/<main branch>`, fetched
first), then logs each stage commit's rebased twin as `commit=` on its card and
`rebase=` on the run card. Run it from the run's worktree or the main checkout.
Output:
  rebase=done head=<sha> mapped=<n>
      The branch is rebased; `mapped` commit= lines were appended
  rebase=unchanged head=<sha>
      The newest logged commit is already on the branch; nothing logged
  rebase=conflict paths=<n> hint=resolve the conflicts, stage them with git add
  or git rm, run GIT_EDITOR=true git rebase --continue, then run the same ferrule
  run rebase again
      Git stopped on a conflict and the rebase stays in progress: resolve it,
      stage the files, run `GIT_EDITOR=true git rebase --continue`, then run
      this command again (exit 1)
Logged on the run card: `rebase=<head> base=<base> from=<old head>`.
Refusals (stdout, exit 1):
  rebase=refused reason=card hint=check the slug with `ferrule board list`
      No card for the slug
  rebase=refused reason=branch want=<branch>
      The worktree's HEAD is not the run's branch
  rebase=refused reason=no-worktree
      The run's worktree is missing
  rebase=refused reason=dirty
      The worktree holds uncommitted changes, untracked files included
  rebase=refused reason=state card=<id>
      A stage card is inprogress
  rebase=refused reason=in-progress
      A rebase is already in progress in the worktree
  rebase=refused reason=no-merge-base
      The branch point with the base cannot be found
  rebase=refused reason=count old=<n> new=<n>
      The rebased branch has another number of commits than the logged ones
  rebase=refused reason=mismatch at=<n> old=<sha> new=<sha>
      A rebased commit's message differs from the logged one
  rebase=refused reason=git detail=\"<git's error>\"
      A git command failed
Refusals end in ` hint=<fix>`.
Exit codes: 0 done or unchanged, 1 conflict or refused, 2 usage, 4 internal error.
Dev build: the open that takes a copy of an older shared board prints to stderr
`ferrule=dev-copy copy=<path> shared=<n> schema=<m>`; the exit code is unchanged.
See: ferrule guide show stages";

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
  rebuild=refused reason=no-pr         The host has no such PR
  rebuild=refused reason=no-stages     The PR body has no `## Stages` list
  rebuild=refused reason=no-branch     The PR's branch is unknown or cannot be
                                       fetched from origin
  rebuild=refused reason=board-exists  The run still has a card, stage cards
                                       or run directory; use reconcile
  rebuild=refused reason=no-issue      The tracker has no such issue
  rebuild=refused reason=sha sha=<sha> A ticked stage's commit is not in the
                                       repository
  rebuild=refused reason=no-sha stage=s<n>
                                       A ticked stage line has no `(<sha>)`
  rebuild=refused reason=bad-branch branch=<branch>
                                       The branch is not `<prefix>/<slug>`; the
                                       slug (after the first `/`) must be
                                       letters, digits, -, _ and single dots
  rebuild=refused reason=bad-stage stage=<id>
                                       A `## Stages` line is not marked
                                       `<!-- s<n> -->`, repeats a stage, or
                                       blocks on one that is not `s<n>`
  rebuild=refused reason=worktree      `git worktree add` failed
  rebuild=refused reason=host detail=\"<error>\"
                                       The host or tracker failed (auth,
                                       network, an answer it could not read)
Refusals end in ` hint=<fix>`.
Exit codes: 0 done, 1 refused, 2 usage, 4 internal error.
Dev build: the open that takes a copy of an older shared board prints to stderr
`ferrule=dev-copy copy=<path> shared=<n> schema=<m>`; the exit code is unchanged.
See: ferrule guide show resume";

/// Shown after `ferrule run cleanup --help`.
const CLEANUP_AFTER_LONG_HELP: &str = "\
Output:
  cleanup=done archive=<id>        The metrics line is recorded and the cards
                                   and run files are archived as <id>
                                   (`ferrule run export` writes it back), in
                                   one transaction; then the worktree, local
                                   branch and run directory are removed
  cleanup=done archive=<id> issue=<n>
                                   The run references issue <n>: the one it
                                   filed, else the one `run init --issue` named
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
  cleanup=refused reason=no-ci                No ci= logged; `ferrule ci wait`,
                                              or on the user's word only log
                                              ci-waived with a reason
  cleanup=refused reason=ci-red               The last ci= is not 0 and no later
                                              ci-waived line exists
  cleanup=refused reason=issue-open           A feature run opened no PR and its
                                              issue is still open; finish the run or
                                              close the issue
  cleanup=refused reason=issue-unreadable     A feature run opened no PR and its
                                              issue's state could not be read;
                                              on the user's word only, log
                                              `issue-waived` with a reason
CI is not asked of a request run (`kind=request`), an epic, or a run
without a PR whose issue is not open (no issue, or a closed one); its metrics
line records ci=-. A run with a PR needs a green or waived CI result. A
ci-waived line also lets through a run with no ci= (its metrics line records
ci=-); a ci= logged after a waiver decides again.
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
Dev build: the open that takes a copy of an older shared board prints to stderr
`ferrule=dev-copy copy=<path> shared=<n> schema=<m>`; the exit code is unchanged.
See: ferrule guide show cleanup";

/// Shown after `ferrule run abandon --help`.
const ABANDON_AFTER_LONG_HELP: &str = "\
Output:
  abandon=plan                     Without --confirm, nothing changed; one line
                                   per effect follows, in this order:
                                     uncommitted <path>
                                     unpushed <sha> <subject>
                                     close-pr <n>
                                     delete-remote-branch <branch>
                                     delete-branch <branch>
                                     remove-worktree <path>
                                   then `hint=on the user's word only:
                                   ferrule run abandon <slug> --confirm`. A run
                                   with no PR or no remote branch lists neither
  abandon=done archive=<id>        With --confirm: the PR got a comment and was
                                   closed, the remote branch was deleted, the
                                   cards and run files were archived as <id> as
                                   abandoned, and the worktree and local branch
                                   were removed
  abandon=done archive=<id> issue=<n>
                                   The run references issue <n>, left open and
                                   unchanged
  abandon=done archive=<id> resumed
                                   An earlier abandon archived the run but failed
                                   removing the worktree or branch; this one
                                   finished those
  abandon=already archive=<id>     The run is fully abandoned; nothing changed
The user's word: give --confirm only when the user said to abandon the run;
without it nothing changes.
Refusals (nothing is changed):
  abandon=refused reason=card                No such run, or one already
                                             cleaned up
  abandon=refused reason=epic                An epic's parent run; abandon its
                                             slice runs one by one
  abandon=refused reason=merged pr=<n>       The PR is merged; end the run with
                                             `ferrule run cleanup`
  abandon=refused reason=pr-unreadable       The PR's state could not be read
                                             (` detail=\"<host error>\"`); check
                                             the host login, then rerun
  abandon=refused reason=remote-unreadable branch=<branch>
                                             Origin could not be asked; check it
                                             is reachable, then rerun
Refusals after a step was done (rerun; only the steps still missing are done):
  abandon=refused reason=close-pr-failed         The PR could not be closed;
                                                 close it by hand
  abandon=refused reason=remote-delete-failed    The remote branch could not be
                                                 deleted; ask the user
  abandon=refused reason=worktree-remove-failed  The worktree could not be
                                                 removed; ask the user
  abandon=refused reason=branch-delete-failed    The local branch could not be
                                                 deleted; ask the user
The last four carry ` detail=\"<error>\"`. Refusals end in ` hint=<fix>`.
Exit codes: 0 done or planned, 1 refused, 4 internal error.
Dev build: the open that takes a copy of an older shared board prints to stderr
`ferrule=dev-copy copy=<path> shared=<n> schema=<m>`; the exit code is unchanged.
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
                     an epic or a waiver with no ci=
Without a slug: every recorded line, oldest run first (nothing if none);
--last <n> keeps the latest n.
Refusals:
  metrics=refused reason=no-ci     No ci= logged; run `ferrule ci wait <slug>`,
                                   or on the user's word only log ci-waived
  metrics=refused reason=ci-red    The last ci= is not 0 and no later
                                   ci-waived line exists
  metrics=refused reason=card      No card for the slug; check it with
                                   `ferrule board list`
The reason lines end in ` hint=<fix>`.
Exit codes: 0 ok, 1 refused, 4 internal error.
Dev build: the open that takes a copy of an older shared board prints to stderr
`ferrule=dev-copy copy=<path> shared=<n> schema=<m>`; the exit code is unchanged.
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
  gauntlet=<pass|fail> ...       The parent's last gauntlet line, `-` if none
                                 or if a rebase= was logged after it.
  ci=<code|->                    The parent's last `ci=`.
  last=<card> <at> <event> ...   The run's last logged line; left out when
                                 the run has logged nothing.
Refusals (exit 1):
  status=refused reason=card hint=check the slug with `ferrule board list`
Exit codes: 0 ok, 1 refused, 4 internal error.
Dev build: the open that takes a copy of an older shared board prints to stderr
`ferrule=dev-copy copy=<path> shared=<n> schema=<m>`; the exit code is unchanged.
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
Dev build: the open that takes a copy of an older shared board prints to stderr
`ferrule=dev-copy copy=<path> shared=<n> schema=<m>`; the exit code is unchanged.";

/// Shown after `ferrule run log --help`.
const LOG_AFTER_LONG_HELP: &str = "\
Output: what `git log --oneline <base>..HEAD` prints in the run's worktree,
newest first; with --stat each commit's files too. base is main_branch of
ferrule.toml unless --base names another commit.
Refusals (exit 1): the `log=refused reason=card|no-worktree|base hint=…` lines
of `ferrule run diff --help`.
Exit codes: 0 ok, 1 refused, 4 internal error.
Dev build: the open that takes a copy of an older shared board prints to stderr
`ferrule=dev-copy copy=<path> shared=<n> schema=<m>`; the exit code is unchanged.";

/// Shown after `ferrule run plan-diff --help`.
const PLAN_DIFF_AFTER_LONG_HELP: &str = "\
Output: a unified diff from plan.prev.md (plan.md as the last plan review
saw it; gate 2's review round and `stage replan start` save it) to plan.md,
both in the run's directory.
  plan-diff=same                 plan.md has not changed since.
Refusals (exit 1):
  plan-diff=refused reason=no-plan hint=the planner has not written plan.md yet
  plan-diff=refused reason=no-prev hint=no plan review has snapshotted plan.md; read it whole
Exit codes: 0 ok, 1 refused, 4 internal error.";

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
Dev build: the open that takes a copy of an older shared board prints to stderr
`ferrule=dev-copy copy=<path> shared=<n> schema=<m>`; the exit code is unchanged.
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
        slug: Slug,
        /// The issue the user named: a GitHub number or URL, or a Jira key.
        #[arg(long)]
        issue: Option<ItemRef>,
        /// Mark the run as a `/fe-request` run: no slice is detected.
        #[arg(long)]
        request: bool,
        /// Start a slice whose issue has open blockers, after the user approved.
        #[arg(long)]
        start_blocked: bool,
    },
    /// Reconcile a run's cards with git and the host before resuming.
    ///
    /// Prints one row per check; run it before an interrupted run goes on.
    #[command(after_long_help = RECONCILE_AFTER_LONG_HELP)]
    Reconcile {
        /// The run's slug.
        slug: Slug,
        /// Check git only: skip the divergence check and the issue, PR and PR-body rows.
        #[arg(long)]
        local: bool,
    },
    /// Open or close the run's live check.
    #[command(after_long_help = LIVE_CHECK_AFTER_LONG_HELP)]
    LiveCheck {
        /// The run's slug.
        slug: Slug,
        /// Open (`start`) or close (`end`) the check.
        #[arg(value_enum)]
        action: LiveAction,
    },
    /// Rebase a run's branch onto its base and map the stage commits.
    #[command(after_long_help = REBASE_AFTER_LONG_HELP)]
    Rebase {
        /// The run's slug.
        slug: Slug,
        /// The base to rebase onto [default: origin/<main branch>, fetched first].
        #[arg(long)]
        onto: Option<String>,
    },
    /// Recreate a lost board from the issue, its draft PR and git.
    ///
    /// Also adds the worktree back.
    #[command(after_long_help = REBUILD_AFTER_LONG_HELP)]
    Rebuild {
        /// The tracking issue: a GitHub number or URL, or a Jira key.
        issue: ItemRef,
        /// The run's draft PR: a number or URL.
        pr: ItemRef,
    },
    /// Abandon a run the user gives up.
    ///
    /// Without --confirm, lists what it would do and changes nothing. With
    /// it, comments on and closes the PR, deletes the remote and local
    /// branches, removes the worktree and archives the cards as abandoned;
    /// the issue stays open. Give --confirm only on the user's word.
    #[command(after_long_help = ABANDON_AFTER_LONG_HELP)]
    Abandon {
        /// The run's slug.
        slug: Slug,
        /// Act: close the PR, delete the branches, archive. Only on the
        /// user's word.
        #[arg(long)]
        confirm: bool,
    },
    /// Clean up a finished run.
    ///
    /// Removes the worktree and the local branch and archives the cards and
    /// files. Refuses while the worktree holds unpushed work, while CI is
    /// not green (a request run, an epic and a feature run without a PR whose issue
    /// is not open need no CI result), or while a feature run without a PR has an
    /// issue that is open or cannot be read.
    #[command(after_long_help = CLEANUP_AFTER_LONG_HELP)]
    Cleanup {
        /// The run's slug.
        slug: Slug,
    },
    /// Record and print a run's metrics line.
    ///
    /// Needs the run's CI green or waived. Without a slug, prints every line recorded.
    #[command(after_long_help = METRICS_AFTER_LONG_HELP)]
    Metrics {
        /// The run's slug; without one, every line recorded.
        slug: Option<Slug>,
        /// Without a slug: only the latest n lines recorded.
        #[arg(long, value_name = "N", conflicts_with = "slug")]
        last: Option<usize>,
    },
    /// Print where a run stands: cards, worktree, gauntlet, CI, last event.
    #[command(after_long_help = STATUS_AFTER_LONG_HELP)]
    Status {
        /// The run's slug.
        slug: Slug,
    },
    /// Print the run branch's diff against main, in the run's worktree.
    #[command(after_long_help = DIFF_AFTER_LONG_HELP)]
    Diff {
        /// The run's slug.
        slug: Slug,
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
        slug: Slug,
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
        slug: Slug,
    },
    /// Write an archived run back out as files.
    #[command(after_long_help = EXPORT_AFTER_LONG_HELP)]
    Export {
        /// The run's slug.
        slug: Slug,
        /// The archive id `run cleanup` printed [default: the latest].
        #[arg(long)]
        archive: Option<i64>,
        /// Target directory [default: .ferrule/export/<slug>-<archive>].
        #[arg(long)]
        to: Option<PathBuf>,
    },
}

pub fn run(command: &RunCommand, out: &mut impl Write) -> io::Result<u8> {
    match command {
        RunCommand::Init {
            slug,
            issue,
            request,
            start_blocked,
        } => start::init(
            slug,
            ferrule_core::run::gate::Init {
                issue: issue.as_ref(),
                request: *request,
                start_blocked: *start_blocked,
            },
            out,
        ),
        RunCommand::Reconcile { slug, local } => start::reconcile(slug, *local, out),
        RunCommand::LiveCheck { slug, action } => start::live_check(slug, *action, out),
        RunCommand::Rebase { slug, onto } => start::rebase(slug, onto.as_deref(), out),
        RunCommand::Rebuild { issue, pr } => start::rebuild(issue, pr, out),
        RunCommand::Abandon { slug, confirm } => finish::abandon(slug, *confirm, out),
        RunCommand::Cleanup { slug } => finish::cleanup(slug, out),
        RunCommand::Export { slug, archive, to } => {
            finish::export(slug, *archive, to.as_deref(), out)
        }
        RunCommand::Metrics { slug, last } => finish::metrics(slug.as_deref(), *last, out),
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
            let pick = inspect::Pick {
                paths: &paths,
                stat: *stat,
                base: base.as_deref(),
            };
            inspect::diff(slug, &pick, *uncommitted, out)
        }
        RunCommand::Log { slug, stat, base } => {
            let pick = inspect::Pick {
                paths: &[],
                stat: *stat,
                base: base.as_deref(),
            };
            inspect::log(slug, &pick, out)
        }
        RunCommand::PlanDiff { slug } => inspect::plan_diff(slug, out),
        RunCommand::Status { slug } => inspect::status(slug, out),
    }
}
