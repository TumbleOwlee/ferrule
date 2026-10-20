//! `ferrule ci`: checks of the code, local and remote.

use std::io::{self, Write};

use clap::Subcommand;
use ferrule_core::config::Config;
use ferrule_core::{project, remote};

use super::board::sql;
use super::run;

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
  gauntlet=fail step=cd reason=no-worktree
      The run has no worktree; `ferrule gate approve <slug> gate2` adds it.
  gauntlet=fail step=config reason=no-steps
      ferrule.toml has no [[gauntlet.step]]; add the project's checks.
The no-worktree and no-steps lines end in ` hint=<fix>`.
Exit codes: 0 pass, 1 fail.
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
  post=refused reason=card         The run has no PR recorded; open it with
                                   `ferrule pr open <slug>`
  (stderr: the host's error)       The checks could not be read (exit 1)
The ci=refused line ends in ` hint=<fix>`.
Exit codes: 0 ok, 1 refused.
See: ferrule guide show gate4";

const FAILED_AFTER_LONG_HELP: &str = "\
Output (stdout): the error lines of the failed job's log, one per line, each
followed by `--context` lines; nothing when the log has none. Exit 0.
Errors (stderr, exit 1): `ci failed: <error>` when there is no failed run, no
failing job matching `--job`, or the host call fails.
Exit codes: 0 ok, 1 refused.
Backend: reads the run's logs from the configured CI host (`gh run view` on
GitHub, pipelines on Bitbucket).
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
        slug: String,
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
        slug: String,
    },
    /// Print the error lines of a failed CI run.
    ///
    /// The branch's latest failed run and its first failing job unless
    /// `--run` and `--job` name others. `ci wait` prints the same lines when
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
        CiCommand::Gauntlet { slug, force } => run::gauntlet(slug, *force, out),
        CiCommand::Wait { slug } => run::ci(slug, out),
        CiCommand::Failed {
            branch,
            run,
            job,
            context,
        } => {
            let config = Config::load(&project::main_checkout()).map_err(sql)?;
            let query = remote::Query {
                run: run.as_deref(),
                job: job.as_deref(),
                context: *context,
            };
            super::pr::failed(&config, branch.as_deref(), query, out)
        }
    }
}
