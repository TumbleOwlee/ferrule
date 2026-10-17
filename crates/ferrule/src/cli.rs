use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

/// Swiss-knife CLI for a spec-driven agent workflow.
#[derive(Debug, Parser)]
#[command(name = "ferrule", version, propagate_version = true)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Spec files and any markdown file.
    #[command(subcommand)]
    Spec(SpecCommand),
}

#[derive(Debug, Subcommand)]
pub enum SpecCommand {
    /// Print markdown sections: each heading line through the next heading
    /// of equal or shallower depth. Batch every heading needed from one file
    /// into a single call.
    Section(SectionArgs),
    /// List a markdown file's headings exactly as `spec section` expects them.
    Sections(SectionsArgs),
    /// Find spec entries by fixed ID and print each as `file:line:text`.
    Id(IdArgs),
    /// Spec entries that mention a keyword (case-insensitive substring).
    Related(RelatedArgs),
    /// Fixed IDs this branch defines that the target ref now defines too.
    Collisions(CollisionsArgs),
}

#[derive(Debug, Args)]
pub struct SectionArgs {
    /// Match the first heading that starts with each given heading.
    #[arg(long)]
    pub prefix: bool,
    /// Headings, verbatim as `spec sections` lists them.
    #[arg(required = true, num_args = 1..)]
    pub headings: Vec<String>,
    pub file: PathBuf,
}

#[derive(Debug, Args)]
pub struct SectionsArgs {
    #[arg(required = true)]
    pub files: Vec<PathBuf>,
}

#[derive(Debug, Args)]
pub struct IdArgs {
    /// Spec directory [default: docs/specs under the project root].
    #[arg(long)]
    pub dir: Option<PathBuf>,
    /// Fixed IDs, `-R-` and `-E-` mixed; batch every ID needed into one call.
    #[arg(required = true)]
    pub ids: Vec<String>,
}

#[derive(Debug, Args)]
pub struct RelatedArgs {
    #[arg(required = true)]
    pub keywords: Vec<String>,
}

#[derive(Debug, Args)]
pub struct CollisionsArgs {
    /// The run's worktree, already fetched.
    pub worktree: PathBuf,
    /// The branch the run merges into.
    #[arg(default_value = "origin/main")]
    pub reference: String,
}
