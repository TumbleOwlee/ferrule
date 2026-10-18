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

/// Swiss-knife CLI for a spec-driven agent workflow.
///
/// Every command prints `key=value` status lines. Exit codes: 0 done, 1
/// refused, 2 usage, 3 content (a body or file that breaks a rule).
#[derive(Debug, Parser)]
#[command(name = "ferrule", version)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
    /// How documents print: styled for a terminal, or markdown.
    #[arg(long, global = true, value_enum, default_value_t = Format::Auto)]
    pub format: Format,
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
    /// Read spec and other markdown files.
    #[command(subcommand)]
    Spec(SpecCommand),
    /// Read and change the task board.
    #[command(subcommand)]
    Card(CardCommand),
    /// Drive a run, from its start to its clean-up.
    #[command(subcommand)]
    Run(RunCommand),
    /// Show the user what a stop needs.
    #[command(subcommand)]
    Prompt(PromptCommand),
    /// Read, assemble and file the tracking issue.
    #[command(subcommand)]
    Issue(IssueCommand),
    /// Read, assemble, push and publish the pull request.
    #[command(subcommand)]
    Pr(PrCommand),
    /// Print the workflow's instructions, one topic at a time.
    ///
    /// A topic is cut to the run's lane, its finished steps are marked, and
    /// `{slug}`, `{run_dir}` and the other placeholders are filled from the
    /// run's card.
    Guide(GuideArgs),
    /// Set up or upgrade ferrule in this repository.
    ///
    /// Writes `.ferrule/`, the agent host's files and the git hooks, and
    /// records what it wrote in `.ferrule/ferrule.lock`. A rerun upgrades
    /// unedited files and writes `<file>.ferrule-new` beside edited ones.
    Install(InstallArgs),
    /// Remove what `install` added.
    Uninstall {
        /// Also remove `.ferrule/`: the board and every run's files.
        #[arg(long)]
        purge: bool,
    },
    /// Handle a Claude Code or git hook.
    #[command(subcommand)]
    Hook(HookCommand),
}

/// A stage id, `s<n>`.
pub fn stage_id(s: &str) -> Result<u32, String> {
    s.strip_prefix('s')
        .and_then(|n| n.parse().ok())
        .ok_or_else(|| format!("expected s<n>, got {s}"))
}
