use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

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
    /// Render one part of a run's spec-diff.md as markdown.
    Render(RenderArgs),
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

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum RenderPart {
    /// `## Scenarios` with Given/When/Then sub-items.
    Scenarios,
    /// `## Requirements` list, local IDs written back.
    Requirements,
    /// `### Requirements` table and `### Other spec changes`.
    Entries,
    /// Dated revision items ending in who decided them.
    Changes,
    /// `### Deferred` list.
    Deferred,
}

#[derive(Debug, Args)]
pub struct RenderArgs {
    pub spec_diff: PathBuf,
    pub part: RenderPart,
    /// Keep only the scenarios and entries named, by local or fixed ID,
    /// space-separated.
    #[arg(long, value_name = "IDS")]
    pub only: Option<String>,
    /// With --only: 1 keeps "Other spec changes" rows that cite no ID.
    #[arg(long, default_value_t = 0, value_parser = clap::value_parser!(u8).range(0..=1))]
    pub first: u8,
    /// PR form of the entries table: an Issue column, and the requirements
    /// reworded after filing, read from the run's cards under this board.
    #[arg(long, value_name = "BOARD")]
    pub issue_ids: Option<PathBuf>,
}
