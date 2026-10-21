//! `ferrule ci`: checks of the code, local and remote.

use ferrule_core::card::board::Slug;
use std::io::{self, Write};

use clap::Subcommand;
use ferrule_core::card::board;
use ferrule_core::config::Config;
use ferrule_core::refusal::Refusal;
use ferrule_core::run::gauntlet::{self, Outcome};
use ferrule_core::{project, remote};

use super::board::store;
use super::{exit, internal, layout, refuse};

/// Shown after `ferrule ci gauntlet --help`.
const GAUNTLET_AFTER_LONG_HELP: &str = "\
Output (the steps' own output goes to .ferrule/runs/<slug>/gauntlet.log):
  gauntlet=pass cov=<pct> sha=<sha> [cached] [dirty]
      Every step passed; `cov` is the line coverage of the step marked
      `coverage = \"llvm-cov\"`, `?` when none is. `cached`: this exact tree
      passed before with the same steps and nothing ran; --force runs it
      anyway. A failing run forgets the cached pass. `dirty`: the worktree
      holds changes beyond HEAD, so the pass is not of the commit; it is
      never cached, and `ferrule pr push` refuses it. Not logged on the
      card: `ferrule pr push` logs `gauntlet=pass`.
Failures:
  gauntlet=fail step=<name> sha=<sha> log=<path>
      The step exited non-zero; read the log, fix, rerun.
  gauntlet=fail step=<name> sha=<sha> reason=timeout log=<path>
      The step ran past its timeout and was killed.
Refusals (nothing ran):
  gauntlet=refused reason=card hint=check the slug with `ferrule board list`
      There is no board yet, so no run; nothing is created.
  gauntlet=refused reason=no-worktree hint=<fix>
      The run has no worktree; `ferrule gate approve <slug> gate2` adds it.
  gauntlet=refused reason=no-steps hint=<fix>
      ferrule.toml has no [[gauntlet.step]]; add the project's checks.
Exit codes: 0 pass, 1 fail, 4 internal error.
See: ferrule guide show verify";

/// Shown after `ferrule ci wait --help`.
const WAIT_AFTER_LONG_HELP: &str = "\
Output:
  ci=<code> [failed=<jobs>]        The checks' exit code (0 is green) and,
                                   when red, the comma-separated failed jobs
  <error line>...                  When red: the failed run's error lines
  next=, run:, do:, guide: lines   The next step, as `ferrule gate next <slug>
                                   ci ci=<code>` prints it; see its help
The output of the wait is saved in .ferrule/runs/<slug>/ci.log.
Refusals:
  ci=refused reason=card           No such run; check the slug with
                                   `ferrule board list`
  ci=refused reason=no-pr          The run has no PR recorded; open it with
                                   `ferrule pr open <slug>`
  ci=refused reason=no-checks      The PR reports no checks (CI has not started
                                   or the repo has none); nothing is logged,
                                   rerun once checks appear
  (stderr: the host's error)       The checks could not be read (exit 1)
The ci=refused lines end in ` hint=<fix>`.
Exit codes: 0 ok, 1 refused, 4 internal error.
See: ferrule guide show gate4";

const FAILED_AFTER_LONG_HELP: &str = "\
Output (stdout): the error lines of the failed job's log, one per line, each
followed by `--context` lines; nothing when the log has none. Exit 0.
Errors (stderr, exit 1): `ci failed: <error>` when there is no failed run, no
failing job matching `--job`, or the host call fails.
Exit codes: 0 ok, 1 refused, 4 internal error.
Backend: reads the run's logs from the configured code host (`gh run view` on
GitHub, pipelines on Bitbucket).
On GitHub only a run of the branch's head commit counts: `No failed run found
for branch '<b>' at <sha>` when only an older commit failed.
See: ferrule guide show gate4";

#[derive(Debug, Subcommand)]
pub enum CiCommand {
    /// Run the gauntlet in the run's worktree.
    ///
    /// The steps are `[[gauntlet.step]]` of `ferrule.toml`. Prints one
    /// status line; the output goes to the run's gauntlet.log.
    #[command(after_long_help = GAUNTLET_AFTER_LONG_HELP)]
    Gauntlet {
        /// The run's slug.
        slug: Slug,
        /// Run even when this tree passed before.
        #[arg(long)]
        force: bool,
    },
    /// Wait for the PR's checks and print the next step.
    ///
    /// Logs `ci=`; on red, also prints the failed run's error lines.
    #[command(after_long_help = WAIT_AFTER_LONG_HELP)]
    Wait {
        /// The run's slug.
        slug: Slug,
    },
    /// Print the error lines of a failed CI run.
    ///
    /// The failed run of the branch's head commit (an older commit's
    /// failure is not shown) and its first failing job unless `--run` and
    /// `--job` name others. `ci wait` prints the same lines when
    /// it waits for a run's checks.
    #[command(after_long_help = FAILED_AFTER_LONG_HELP)]
    Failed {
        /// The branch [default: the current branch].
        #[arg(conflicts_with = "run")]
        branch: Option<String>,
        /// A run id (GitHub) or pipeline uuid or number (Bitbucket).
        #[arg(long, value_name = "ID")]
        run: Option<String>,
        /// The failing job (step) whose name contains this, ignoring case.
        #[arg(long, value_name = "NAME")]
        job: Option<String>,
        /// Lines to print after each error line.
        #[arg(long, value_name = "N", default_value_t = 0)]
        context: usize,
    },
}

pub fn run(command: &CiCommand, out: &mut impl Write) -> io::Result<u8> {
    match command {
        CiCommand::Gauntlet { slug, force } => run_gauntlet(slug, *force, out),
        CiCommand::Wait { slug } => wait(slug, out),
        CiCommand::Failed {
            branch,
            run,
            job,
            context,
        } => {
            let config = Config::load(&project::main_checkout()).map_err(internal)?;
            let query = remote::Query {
                run: run.as_deref(),
                job: job.as_deref(),
                context: *context,
            };
            super::pr::failed(&config, branch.as_deref(), query, out)
        }
    }
}

fn run_gauntlet(slug: &str, force: bool, out: &mut impl Write) -> io::Result<u8> {
    let layout = layout();
    let config = Config::load(&layout.root).map_err(internal)?;
    // No board, no run: the gauntlet never creates one.
    let Some(s) = store()? else {
        return refuse(out, &Refusal::card("gauntlet"));
    };
    let wt = s
        .read(|r| r.worktree(&layout, slug, slug))
        .map_err(internal)?;
    let log = layout.run_dir(slug).join("gauntlet.log");
    let outcome = gauntlet::run(
        &s,
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

fn wait(slug: &str, out: &mut impl Write) -> io::Result<u8> {
    use ferrule_core::run::ci;
    let layout = layout();
    let config = Config::load(&layout.root).map_err(internal)?;
    let Some(mut s) = store()? else {
        return refuse(out, &Refusal::card("ci"));
    };
    let w = match ci::wait(&mut s, &layout, &config, slug, board::now) {
        Ok(w) => w,
        Err(e) => {
            if let Some(r) = e.refusal(slug) {
                return refuse(out, &r);
            }
            return match e {
                ci::Error::Publish(e) => super::issue::post_failed(e, "ci", out),
                ci::Error::Next(e) => Err(internal(e)),
                ci::Error::NoPr | ci::Error::NoChecks => Ok(exit::REFUSED),
            };
        }
    };
    write!(out, "ci={}", w.code)?;
    if !w.failed.is_empty() {
        write!(out, " failed={}", w.failed.join(","))?;
    }
    writeln!(out)?;
    for l in w.lines.iter().chain(&w.next) {
        writeln!(out, "{l}")?;
    }
    Ok(exit::OK)
}
