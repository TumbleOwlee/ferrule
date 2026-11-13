//! `ferrule stage`: one stage's work.

use ferrule_core::card::board::Slug;
use std::io::{self, Write};

use clap::{Subcommand, ValueEnum};
use ferrule_core::card::board;
use ferrule_core::refusal::{CARD_HINT, Refusal};
use ferrule_core::run::testproof::{self, ProofRefused};
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
  commit=refused reason=rebasing
                                 A rebase is in progress in the worktree, with
                                 and without --adopt: resolve it and run
                                 `GIT_EDITOR=true git rebase --continue`, then
                                 `ferrule run rebase <slug>`, or `git rebase --abort`
  commit=refused reason=card     No such card; check it with
                                 `ferrule board list`
  commit=refused reason=state state=<s>
                                 The card is in <s>, not inreview; move it with
                                 `ferrule board move <card> inreview`
  commit=refused reason=branch want=<b>
                                 The run's `branch` field is <b> and another
                                 branch is checked out; check out <b>
  commit=refused reason=nothing-staged head=<sha>
                                 Nothing is staged: on a dirty worktree stage
                                 the changes with `git add`; on a clean one
                                 they may already be committed, <sha> is HEAD
  commit=refused reason=test-unproven tests=<a,b>
                                 Rust: new tests with no `ferrule stage red`
                                 failure or `ferrule stage pins` mark on the card
  commit=refused reason=test-unreplaced tests=<a,b>
                                 Rust: removed or rewritten tests no
                                 `--replaced <old>=<new>` names
  commit=refused reason=git      `git commit` failed with changes staged (a
                                 hook refused); git's message is on stderr
With --adopt <sha>, instead of -m, HEAD (committed outside ferrule) is logged
as the stage's commit; no commit is created and history is unchanged:
  commit=done sha=<sha> card=<id>
                                 Logged `commit=<sha>` on the card, as above.
  Checked in order: main, rebasing, card, state and branch as above (the run needs a
  `branch` field), then each line ends in `hint=<fix>`; the two test refusals
  above apply after no-merge-base, before trailers:
  commit=refused reason=dirty    The worktree holds changes
  commit=refused reason=not-head head=<sha>
                                 <sha> is not the branch's HEAD
  commit=refused reason=merge parents=<n>
                                 HEAD is a merge commit
  commit=refused reason=adopted card=<id>
                                 HEAD is already logged as commit= on <id>
  commit=refused reason=unlogged sha=<sha>
                                 A commit before HEAD is logged on no card
  commit=refused reason=no-merge-base
                                 The branch point with the main branch
                                 cannot be found
  commit=refused reason=trailers
                                 The message carries attribution trailers;
                                 amend it if it is not pushed
--adopt conflicts with -m and --no-run.
--replaced <old>=<new> names a removed or rewritten test's replacement
(repeatable, also with --adopt; conflicts with --no-run).
Notes logged on the card after `commit=`: `files-outside=<path>` per path
outside the card's files list, `test-replaced old=<o> new=<n>` per
--replaced, `test-checks=skipped stack=<s>` when the stack has no test checks.
With --no-run the nothing-staged check applies, the new-test and removed-test
checks do not, and no card is touched; the main-branch check applies too.
Attribution trailers are stripped from the message first.
Exit codes: 0 ok, 1 refused, 4 internal error.
Dev build: the open that takes a copy of an older shared board prints to stderr
`ferrule=dev-copy copy=<path> shared=<n> schema=<m>`; the exit code is unchanged.
See: ferrule guide show stages";

/// Shown after `ferrule stage red --help`.
const RED_AFTER_LONG_HELP: &str = "\
Runs the tests matching <test> in the current directory with the stack's test
command (a Rust project: `cargo test --workspace -- <test>`) and logs one
`test-red test=<name>` per test the run reports failed; with --mutation <text>
each line also carries `mutation=<text>`, the proof that the test fails when
the code under it is broken.
Output:
  red=recorded tests=<a,b>         One `test-red` logged per failed test
Refusals (each line ends in `hint=<fix>`):
  red=refused reason=card          No such card; check it with
                                   `ferrule board list`
  red=refused reason=state state=<s>
                                   The card is in <s>, not inprogress or
                                   inreview
  red=refused reason=stack         Not a Rust project: only it records proofs
  red=refused reason=no-failure    The run reported no failed test (a test
                                   that does not compile is none): make it
                                   compile and fail, then run again
  red=refused reason=run           The test command could not start or timed out
Exit codes: 0 ok, 1 refused, 4 internal error.
Dev build: the open that takes a copy of an older shared board prints to stderr
`ferrule=dev-copy copy=<path> shared=<n> schema=<m>`; the exit code is unchanged.
See: ferrule guide show stages";

/// Shown after `ferrule stage pins --help`.
const PINS_AFTER_LONG_HELP: &str = "\
Marks <test> as passing from the start because it pins behaviour the code
already has; <text> names what it pins.
Output:
  pins=recorded test=<t>           Logged `test-pins test=<t> -- <text>`
Refusals (each line ends in `hint=<fix>`):
  pins=refused reason=card         No such card; check it with
                                   `ferrule board list`
  pins=refused reason=state state=<s>
                                   The card is in <s>, not inprogress or
                                   inreview
  pins=refused reason=stack        Not a Rust project: only it records proofs
Exit codes: 0 ok, 1 refused, 4 internal error.
Dev build: the open that takes a copy of an older shared board prints to stderr
`ferrule=dev-copy copy=<path> shared=<n> schema=<m>`; the exit code is unchanged.
See: ferrule guide show stages";

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
Dev build: the open that takes a copy of an older shared board prints to stderr
`ferrule=dev-copy copy=<path> shared=<n> schema=<m>`; the exit code is unchanged.
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
Refusals:
  stage-stop=refused reason=card   No such stage card; check the slug and
                                   stage with `ferrule board list`
The refusal ends in ` hint=<fix>`.
Exit codes: 0 auto or stop, 1 refused, 2 usage, 4 internal error.
Dev build: the open that takes a copy of an older shared board prints to stderr
`ferrule=dev-copy copy=<path> shared=<n> schema=<m>`; the exit code is unchanged.
See: ferrule guide show stage-review";

/// Shown after `ferrule stage tier --help`.
const TIER_AFTER_LONG_HELP: &str = "\
Output (also logged on the stage card as `tier=<tier>`):
  tier=<own|batched|skipped> review=<stage|batch|none> [stages=s<a>,s<b>]
                                   [promoted=<why>]
    review=stage   The stage is reviewed alone
    review=batch   A batch closes here; stages= lists its stages, oldest first
    review=none    No review before gate 3
Promotions (the stage then has its own review):
  promoted=spec-change     A planned batched or skipped stage changes docs/specs
  promoted=batch-blocker   --after-blocker: the previous batch's review found
                           a blocker
Refusals:
  stage-tier=refused reason=card   No such stage card; check the slug and
                                   stage with `ferrule board list`
The refusal ends in ` hint=<fix>`.
Exit codes: 0 decided, 1 refused, 2 usage, 4 internal error.
See: ferrule guide show stages";

/// Shown after `ferrule stage replan --help`.
const REPLAN_AFTER_LONG_HELP: &str = "\
Output:
  replan=started from=s<n>         Logged on the card, plan.md saved as
                                   plan.prev.md; <n> is the lowest stage
                                   without a commit. The parent's gate4
                                   date is removed: it is approved anew
  replan=approved from=s<n> stages=<n>
                                   The uncommitted stage cards from s<n> on
                                   are replaced by the new plan's <n>
                                   stages (a committed one stays); each
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
  replan=refused reason=committed stage=s<n>
                                   approve: the new plan redefines s<n>,
                                   which is committed and stays
Refusals end in ` hint=<fix>`.
Exit codes: 0 ok, 1 refused, 2 usage, 4 internal error.
Dev build: the open that takes a copy of an older shared board prints to stderr
`ferrule=dev-copy copy=<path> shared=<n> schema=<m>`; the exit code is unchanged.
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
        #[arg(short, long = "message", required_unless_present = "adopt")]
        message: Vec<String>,
        /// Commit outside a run: only the branch, nothing-staged and trailer rules apply.
        #[arg(long)]
        no_run: bool,
        /// Log HEAD, committed outside ferrule, as the stage's `commit=`.
        #[arg(long, value_name = "SHA", conflicts_with_all = ["message", "no_run"])]
        adopt: Option<String>,
        /// Name the replacement of a removed or rewritten test, `<old>=<new>`; repeatable.
        #[arg(long, value_name = "OLD=NEW", value_parser = parse_replaced, conflicts_with = "no_run")]
        replaced: Vec<(String, String)>,
    },
    /// Run a test and record each failure it reports.
    ///
    /// The failure is the proof a new test needs before `ferrule stage commit`.
    #[command(after_long_help = RED_AFTER_LONG_HELP)]
    Red {
        /// The stage card, `<slug>.s<n>`.
        card: String,
        /// The test name, or a part of it, the test command filters by.
        test: String,
        /// Record the failure as a mutation proof naming this change.
        #[arg(long)]
        mutation: Option<String>,
    },
    /// Mark a test as pinning existing behaviour.
    ///
    /// Such a test passes from the start; the mark takes the place of a failure.
    #[command(after_long_help = PINS_AFTER_LONG_HELP)]
    Pins {
        /// The stage card, `<slug>.s<n>`.
        card: String,
        /// The test name.
        test: String,
        /// What the test pins.
        #[arg(last = true, required = true)]
        why: Vec<String>,
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
    /// Decide whether a green stage reviewed on its own stops for approval.
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
    /// Decide a green stage's tier and review point.
    ///
    /// Logs the tier on the stage card.
    #[command(after_long_help = TIER_AFTER_LONG_HELP)]
    Tier {
        /// The run's slug.
        slug: Slug,
        /// The stage, `s<n>`.
        #[arg(value_parser = stage_id, value_name = "STAGE")]
        stage: u32,
        /// The previous batch's review found a blocker: this stage gets its
        /// own review.
        #[arg(long)]
        after_blocker: bool,
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
        #[arg(long, value_parser = ferrule_core::run::gate::parse_stops)]
        stops: Option<String>,
    },
}

/// `<old>=<new>`, split at the first `=`.
fn parse_replaced(s: &str) -> Result<(String, String), String> {
    match s.split_once('=') {
        Some((old, new)) if !old.is_empty() && !new.is_empty() => {
            Ok((old.to_owned(), new.to_owned()))
        }
        _ => Err("expected <old>=<new>".to_owned()),
    }
}

pub fn run(command: &StageCommand, out: &mut impl Write) -> io::Result<u8> {
    match command {
        StageCommand::Commit {
            card: Some(card),
            adopt: Some(sha),
            replaced,
            ..
        } => super::board::adopt(card, sha, replaced, out),
        StageCommand::Commit {
            card,
            message,
            replaced,
            ..
        } => super::board::commit(card.as_deref(), message, replaced, out),
        StageCommand::Red {
            card,
            test,
            mutation,
        } => red(card, test, mutation.as_deref(), out),
        StageCommand::Pins { card, test, why } => pins(card, test, &why.join(" "), out),
        StageCommand::Verify { slug, stage, sha } => sha_check(slug, *stage, sha, out),
        StageCommand::Stop {
            slug,
            stage,
            threads,
        } => stage_stop(slug, *stage, *threads, out),
        StageCommand::Tier {
            slug,
            stage,
            after_blocker,
        } => stage_tier(slug, *stage, *after_blocker, out),
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

/// The refusal of `verb` (`red` or `pins`) for `refused`.
fn proof_refusal(verb: &'static str, refused: &ProofRefused) -> Refusal {
    match refused {
        ProofRefused::Card => Refusal::card(verb),
        ProofRefused::State(st) => Refusal::new(verb, "state")
            .field("state", st)
            .hint("move the card to inprogress first: `ferrule board move <card> inprogress`"),
        ProofRefused::Stack => Refusal::new(verb, "stack").hint(
            "only a Rust project records test proofs; set `stack = \"rust\"` in ferrule.toml",
        ),
        ProofRefused::NoFailure => Refusal::new(verb, "no-failure").hint(
            "the run reported no failed test: make the test compile and fail, then run again",
        ),
        ProofRefused::Run => Refusal::new(verb, "run")
            .hint("the test command could not start or timed out: run it by hand"),
    }
}

fn red(card: &str, test: &str, mutation: Option<&str>, out: &mut impl Write) -> io::Result<u8> {
    let here = std::env::current_dir()?;
    let config = super::config()?;
    let Some(mut s) = store()? else {
        return refuse(out, &Refusal::card("red"));
    };
    let got = testproof::red(
        &mut s,
        &here,
        card,
        config.stack.as_deref(),
        test,
        mutation,
        &board::now(),
    )
    .map_err(internal)?;
    match got {
        Ok(tests) => {
            writeln!(out, "red=recorded tests={}", tests.join(","))?;
            Ok(exit::OK)
        }
        Err(r) => refuse(out, &proof_refusal("red", &r)),
    }
}

fn pins(card: &str, test: &str, why: &str, out: &mut impl Write) -> io::Result<u8> {
    let config = super::config()?;
    let Some(mut s) = store()? else {
        return refuse(out, &Refusal::card("pins"));
    };
    let got = testproof::pins(
        &mut s,
        card,
        config.stack.as_deref(),
        test,
        why,
        &board::now(),
    )
    .map_err(internal)?;
    match got {
        Ok(()) => {
            writeln!(out, "pins=recorded test={test}")?;
            Ok(exit::OK)
        }
        Err(r) => refuse(out, &proof_refusal("pins", &r)),
    }
}

fn sha_check(slug: &str, stage: u32, sha: &str, out: &mut impl Write) -> io::Result<u8> {
    let verdict = match store()? {
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
    let config = super::config()?;
    let Some(mut s) = store()? else {
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

fn stage_tier(slug: &str, stage: u32, after_blocker: bool, out: &mut impl Write) -> io::Result<u8> {
    let layout = layout();
    let config = super::config()?;
    let Some(mut s) = store()? else {
        return refuse(out, &Refusal::card("stage-tier"));
    };
    match core_run::tier::decide(
        &mut s,
        &layout,
        &config,
        slug,
        stage,
        after_blocker,
        &board::now(),
    ) {
        Ok(line) => writeln!(out, "{line}")?,
        Err(Error::Card) => return refuse(out, &Refusal::card("stage-tier")),
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
    let Some(mut s) = store()? else {
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
