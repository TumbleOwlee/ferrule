use clap::{Parser, Subcommand, ValueEnum};

use crate::commands::card::CardCommand;
use crate::commands::guide::GuideArgs;
use crate::commands::hook::HookCommand;
use crate::commands::install::InstallArgs;
use crate::commands::issue::IssueCommand;
use crate::commands::pr::PrCommand;
use crate::commands::prompt::PromptCommand;
use crate::commands::run::RunCommand;
use crate::commands::spec::SpecCommand;

/// Shown after `ferrule --help`.
const AFTER_LONG_HELP: &str = "\
Getting started:
  ferrule install              Set up .ferrule/, the agent files and the git hooks
  ferrule guide --list         The workflow's topics, orchestrator and agent
  ferrule guide <topic>        One topic, cut to the current run

Output:
  Status lines are `key=value`, one fact per line. Documents (guide, spec,
  card show|board, issue and pr view|request|body) are styled on a terminal
  and printed as markdown when piped; `--format` overrides that.
  A refusal ends in `hint=<fix>`. Each command's --help lists its outputs
  and refusals. Every command but install and uninstall refuses (stderr)
  with `ferrule=refused reason=version pinned=<v> binary=<v>: <fix>` when
  the binary and the project's version differ; the line names the fix.

Exit codes:
  0  done, or a negative answer that is not an error
  1  refused or not found
  2  usage, or input the rules reject
  3  content the rules refuse: the agent that wrote it fixes it

Environment:
  NO_COLOR      Styled output without colours
  COLUMNS       Width to wrap to when the terminal's is unknown [default: 100]
  FERRULE_NOW   Timestamp written to card logs instead of the clock";

/// One binary for a spec-driven agent workflow.
///
/// The board, specs, issues, pull requests, gates and hooks of a run, as
/// subcommands with the workflow's rules built in.
#[derive(Debug, Parser)]
#[command(
    name = "ferrule",
    version,
    after_long_help = AFTER_LONG_HELP,
    disable_help_flag = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
    /// How documents print: `auto` styles them on a terminal and prints
    /// markdown otherwise, `text` always styles, `md` never does.
    #[arg(
        long,
        global = true,
        value_enum,
        value_name = "auto|text|md",
        default_value_t = Format::Auto,
        hide_possible_values = true,
        hide_default_value = true,
        help_heading = "Global options"
    )]
    pub format: Format,
    /// Print help.
    #[arg(short, long, global = true, action = clap::ArgAction::HelpLong, help_heading = "Global options")]
    pub help: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Format {
    /// Styled when stdout is a terminal, markdown otherwise.
    Auto,
    /// Styled; colours off under `NO_COLOR`.
    Text,
    /// Markdown, as stored.
    Md,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Set up or upgrade ferrule in this repository.
    ///
    /// Writes `.ferrule/`, the agent host's files and the git hooks, and
    /// records what it wrote in `.ferrule/ferrule.lock`. A rerun upgrades
    /// unedited files and writes `<file>.ferrule-new` beside edited ones.
    Install(InstallArgs),
    /// Remove what `install` added.
    ///
    /// Keeps `.ferrule/` (the board and every run's files) unless `--purge`.
    #[command(after_long_help = crate::commands::uninstall::UNINSTALL_AFTER_LONG_HELP)]
    Uninstall {
        /// Also remove `.ferrule/`: the board and every run's files.
        #[arg(long)]
        purge: bool,
    },
    /// Print the workflow's instructions, one topic at a time.
    ///
    /// A topic is cut to the run's lane, its finished steps are marked, and
    /// `{slug}`, `{run_dir}` and the other placeholders are filled from the
    /// run's card.
    Guide(GuideArgs),
    /// Drive a run, from its start to its clean-up.
    #[command(subcommand)]
    Run(RunCommand),
    /// Read and change the task board.
    #[command(subcommand)]
    Card(CardCommand),
    /// Read spec and other markdown files.
    #[command(subcommand)]
    Spec(SpecCommand),
    /// Show the user what a stop needs.
    #[command(subcommand)]
    Prompt(PromptCommand),
    /// Read, assemble and file the tracking issue.
    #[command(subcommand)]
    Issue(IssueCommand),
    /// Read, assemble, push and publish the pull request.
    #[command(subcommand)]
    Pr(PrCommand),
    /// Handle a Claude Code or git hook; called by the hosts, not by hand.
    #[command(subcommand)]
    Hook(HookCommand),
}

/// A stage id, `s<n>`.
pub fn stage_id(s: &str) -> Result<u32, String> {
    s.strip_prefix('s')
        .and_then(|n| n.parse().ok())
        .ok_or_else(|| format!("expected s<n>, got {s}"))
}

/// A stage id, `s<n>`, kept as written.
pub fn stage_name(s: &str) -> Result<String, String> {
    stage_id(s).map(|_| s.to_owned())
}
