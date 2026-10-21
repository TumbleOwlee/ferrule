//! `ferrule stage`: one stage's work.

use ferrule_core::card::board::Slug;
use std::io::{self, Write};

use clap::{Subcommand, ValueEnum};
use ferrule_core::card::board;
use ferrule_core::config::Config;
use ferrule_core::refusal::{CARD_HINT, Refusal};
use ferrule_core::run::{self as core_run, Error, gate};

use super::board::store;
use super::{exit, internal, layout, refuse};
use crate::cli::stage_id;

const COMMIT_AFTER_LONG_HELP: &str = "\
Output:
  commit=done sha=<sha>          Committed; <sha> is the short hash.
  commit=done sha=<sha> card=<id>
                                 Also logged `commit=<sha>` on the card.
  commit=done sha=<sha> card-log=failed
                                 Committed, but the log write failed (details
                                 on stderr): log it with
                                 `ferrule board log <id> commit=<sha>`; exit 1.
Refusals (checked in this order; each line ends in `hint=<fix>`):
  commit=refused reason=main branch=<b>
                                 On the main branch <b>; commit on the run's
                                 branch: `ferrule gate approve <slug> gate2`
                                 creates it and its worktree (cd there); with
                                 --no-run, check out another branch first
  commit=refused reason=card     No such card; check it with
                                 `ferrule board list`
  commit=refused reason=state state=<s>
                                 The card is in <s>, not inreview; move it with
                                 `ferrule board move <card> inreview`
  commit=refused reason=branch want=<b>
                                 The run's `branch` field is <b> and another
                                 branch is checked out; check out <b>
  commit=refused reason=git      `git commit` failed (nothing staged, a hook
                                 refused); git's message is on stderr
With --no-run only the main-branch check applies and no card is touched.
Attribution trailers are stripped from the message first.
Exit codes: 0 ok, 1 refused, 4 internal error.
See: ferrule guide show implementer";

/// Shown after `ferrule stage verify --help`.
const VERIFY_AFTER_LONG_HELP: &str = "\
Output:
  sha=ok                           The commit is in the worktree, on the run's
                                   branch, and the card's last commit=
Refusals:
  sha=mismatch reason=card         No such run or stage; check the slug and
                                   stage with `ferrule board list`
  sha=mismatch reason=missing      The worktree has no such commit; ask the
                                   implementer for the real sha
  sha=mismatch reason=off-branch   The run's branch lacks the commit
  sha=mismatch reason=card-commit  The stage card's last commit= is another
                                   commit; commit with `ferrule stage commit`
Refusals end in ` hint=<fix>`.
Exit codes: 0 ok, 1 mismatch, 2 usage, 4 internal error.
See: ferrule guide show stages";

/// Shown after `ferrule stage stop --help`.
const STOP_AFTER_LONG_HELP: &str = "\
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
  stage-stop=refused reason=card   No such stage card; check the slug and
                                   stage with `ferrule board list`
The refusal ends in ` hint=<fix>`.
Exit codes: 0 auto or stop, 1 refused, 2 usage, 4 internal error.
See: ferrule guide show stage-review";

/// Shown after `ferrule stage replan --help`.
const REPLAN_AFTER_LONG_HELP: &str = "\
Output:
  replan=started from=s<n>         Logged on the card, plan.md saved as
                                   plan.prev.md; <n> is the lowest stage
                                   without a commit. The parent's gate3 and
                                   gate4 dates are removed: both are
                                   approved anew
  replan=approved from=s<n> stages=<n>
                                   The stage cards from s<n> on are replaced
                                   by the new plan's <n> stages; each
                                   `spec-delta <id> added origin=<o>` line of
                                   the dropped stages is logged again on the
                                   parent, so the ids stay retired
  pr-body=pushed|unchanged         After approve: the draft PR's body synced
  pr-body=refused reason=stage-ref stage-ref=s<n> hint=<fix>
                                   The body names stage s<n> that has no card
  pr-body=failed                   The sync failed (details on stderr)
Refusals:
  replan=refused reason=card       No such run; check the slug with
                                   `ferrule board list`
  replan=refused reason=move       start: the parent cannot move to inprogress
  replan=refused reason=no-replan  approve without a start after the last approval
  replan=refused reason=no-plan    approve: plan.md is missing
  replan=refused reason=no-stages  approve: the plan has no stage from s<n> on
Refusals end in ` hint=<fix>`.
Exit codes: 0 ok, 1 refused, 2 usage, 4 internal error.
See: ferrule guide show replan";

#[derive(Debug, Subcommand)]
pub enum StageCommand {
    /// Commit the staged changes of a stage in review.
    ///
    /// Refuses the main branch and a branch other than the run's, strips
    /// attribution trailers from the message and logs `commit=` on the card.
    #[command(after_long_help = COMMIT_AFTER_LONG_HELP)]
    Commit {
        /// The stage card, `<slug>.s<n>`; omitted with `--no-run`.
        #[arg(required_unless_present = "no_run", conflicts_with = "no_run")]
        card: Option<String>,
        /// The commit message; repeat for more paragraphs.
        #[arg(short, long = "message", required = true)]
        message: Vec<String>,
        /// Commit outside a run: only the branch and trailer rules apply.
        #[arg(long)]
        no_run: bool,
    },
    /// Verify a stage's reported commit before anything is pushed.
    ///
    /// The commit exists in the worktree, the run's branch holds it, and
    /// it is the stage card's last `commit=`.
    #[command(after_long_help = VERIFY_AFTER_LONG_HELP)]
    Verify {
        /// The run's slug.
        slug: Slug,
        /// The stage, `s<n>`.
        #[arg(value_parser = stage_id, value_name = "STAGE")]
        stage: u32,
        /// The commit the implementer reported.
        sha: String,
    },
    /// Decide whether a green full-lane stage stops for approval.
    ///
    /// Logs the outcome on the stage card.
    #[command(after_long_help = STOP_AFTER_LONG_HELP)]
    Stop {
        /// The run's slug.
        slug: Slug,
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
        slug: Slug,
        /// Which half of the re-plan.
        step: ReplanStep,
        /// With `approve`: stages that stop for approval beyond the plan's,
        /// e.g. `s1,s3`.
        #[arg(long)]
        stops: Option<String>,
    },
}

pub fn run(command: &StageCommand, out: &mut impl Write) -> io::Result<u8> {
    match command {
        StageCommand::Commit { card, message, .. } => {
            super::board::commit(card.as_deref(), message, out)
        }
        StageCommand::Verify { slug, stage, sha } => sha_check(slug, *stage, sha, out),
        StageCommand::Stop {
            slug,
            stage,
            threads,
        } => stage_stop(slug, *stage, *threads, out),
        StageCommand::Replan { slug, step, stops } => replan(slug, *step, stops.as_deref(), out),
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ReplanStep {
    /// Log `replan from=` and snapshot the plan.
    Start,
    /// Swap the stage cards from `from` on for the new plan's.
    Approve,
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

fn sha_check(slug: &str, stage: u32, sha: &str, out: &mut impl Write) -> io::Result<u8> {
    let verdict = match store(false)? {
        Some(s) => s
            .read(|r| core_run::sha_check(r, &layout(), slug, &format!("s{stage}"), sha))
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

fn stage_stop(slug: &str, stage: u32, threads: u32, out: &mut impl Write) -> io::Result<u8> {
    let layout = layout();
    let config = Config::load(&layout.root).map_err(internal)?;
    let Some(mut s) = store(false)? else {
        return refuse(out, &Refusal::card("stage-stop"));
    };
    match core_run::stage_stop(
        &mut s,
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

fn replan(
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
        ReplanStep::Start => gate::replan_start(&mut s, &layout, slug, &now)
            .map(|from| format!("replan=started from=s{from}")),
        ReplanStep::Approve => {
            let stops = gate::stop_arg(stops);
            gate::replan_approve(&mut s, &layout, slug, &stops, &now)
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
        Err(e) => super::gate::failed(e, "replan", slug, out),
    }
}
