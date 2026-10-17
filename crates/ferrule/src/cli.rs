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
    /// The task board: cards, their fields, log and state.
    #[command(subcommand)]
    Card(CardCommand),
    /// Load a markdown board (`<board>/<state>/<id>.md`) into the state
    /// database, all cards or none.
    Import(ImportArgs),
    /// A run's mechanical steps: checks against git, stop decisions,
    /// metrics, the gauntlet, clean-up.
    #[command(subcommand)]
    Run(RunCommand),
    /// Claude Code hook handlers: read the hook payload on stdin.
    #[command(subcommand)]
    Hook(HookCommand),
}

#[derive(Debug, Subcommand)]
pub enum HookCommand {
    /// The session-wide `PreToolUse` guard: refuses raw card edits, output
    /// floods, the crashing GitHub views and commits or pushes to main;
    /// strips attribution trailers from what is left.
    PreToolUse,
    /// The reviewer's `PreToolUse` guard: nothing that writes, except the
    /// run's review.md and review.verdict.md.
    Reviewer,
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

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum CardState {
    Open,
    Inprogress,
    Inreview,
    Done,
}

#[derive(Debug, Subcommand)]
pub enum CardCommand {
    /// One frontmatter field, `log:<key>` (the last `<key>=` value in the
    /// log), `pass-sha` (the sha of the last `gauntlet=pass`) or `state` of
    /// a card; nothing when absent.
    Field { card: String, field: String },
    /// Append one validated event to a card's log.
    Log {
        card: String,
        /// `<name>` or `<name>=<value>`, see `ferrule card vocab`.
        event: String,
        /// `[positional…] [key=value…] [-- free text]`.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Move a card to another state, refusing what the board forbids.
    Move { card: String, state: CardState },
    /// The card as markdown: frontmatter and log.
    Show { card: String },
    /// Every card with its state and title.
    Board,
    /// The event vocabulary `card log` accepts.
    Vocab,
    /// Create a card in `open` (or `--state`) with `key=value` fields.
    New {
        card: String,
        #[arg(long, value_enum, default_value_t = CardState::Open)]
        state: CardState,
        fields: Vec<String>,
    },
    /// Set frontmatter fields, `key=value` each; an empty value removes one.
    Set {
        card: String,
        #[arg(required = true)]
        fields: Vec<String>,
    },
}

#[derive(Debug, Args)]
pub struct ImportArgs {
    /// The board directory holding `open/`, `inprogress/`, `inreview/` and
    /// `done/`.
    pub board: PathBuf,
}

/// A stage id, `s<n>`.
fn stage_id(s: &str) -> Result<u32, String> {
    s.strip_prefix('s')
        .and_then(|n| n.parse().ok())
        .ok_or_else(|| format!("expected s<n>, got {s}"))
}

#[derive(Debug, Subcommand)]
pub enum RunCommand {
    /// Verify an implementer's reported commit before anything is pushed:
    /// it exists in the worktree, the run's branch holds it, and it is the
    /// stage card's last `commit=`.
    ShaCheck {
        slug: String,
        #[arg(value_parser = stage_id, value_name = "s<n>")]
        stage: u32,
        sha: String,
    },
    /// Reconcile a run's cards with git before an interrupted run resumes.
    Reconcile { slug: String },
    /// Decide whether a green, clean-reviewed full-lane stage stops for
    /// approval, and log the outcome on its card.
    StageStop {
        slug: String,
        #[arg(value_parser = stage_id, value_name = "s<n>")]
        stage: u32,
        /// Unresolved PR review threads.
        #[arg(long, default_value_t = 0)]
        threads: u32,
    },
}
