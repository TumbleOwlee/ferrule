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
    /// What the user is shown at a stop: quoted lines, files, whole
    /// messages.
    #[command(subcommand)]
    Prompt(PromptCommand),
    /// The tracking issue: read from the tracker, its body assembled.
    #[command(subcommand)]
    Issue(IssueCommand),
    /// The pull request: read from the host, its body assembled, its
    /// review threads, its failed CI run.
    #[command(subcommand)]
    Pr(PrCommand),
    /// The workflow's instructions, one topic at a time, cut to the run's
    /// lane and state.
    Guide(GuideArgs),
    /// Set up or upgrade the workflow in this repository: `.ferrule/`, the
    /// agent host's adapter files, the git hooks.
    Install(InstallArgs),
    /// Remove what `install` added; `.ferrule/` stays unless `--purge`.
    Uninstall {
        /// Also remove `.ferrule/`, the board and every run's files with it.
        #[arg(long)]
        purge: bool,
    },
    /// Claude Code hook handlers: read the hook payload on stdin.
    #[command(subcommand)]
    Hook(HookCommand),
}

#[derive(Debug, Subcommand)]
pub enum IssueCommand {
    /// Title, body and comments as compact text: a GitHub number or URL,
    /// or a Jira key.
    View { issue: String },
    /// A reused issue's original request, for the run's request.md.
    Request {
        issue: String,
        #[command(flatten)]
        part: RequestPart,
    },
    /// The tracking issue's body from the run's artifacts, line 1 `# <title>`;
    /// exit 3 when it names a stage or an agent.
    Body {
        slug: String,
        /// Slice n's own issue instead.
        #[arg(long)]
        slice: Option<u32>,
    },
    /// Post the run's issue, or a slice, split-off or follow-up issue, or
    /// issue-comment.md; record the number. An open issue with the same
    /// title is adopted, never filed twice.
    File {
        slug: String,
        #[command(flatten)]
        target: FileTarget,
        /// The tracking issue reuses issue n: its title and body are
        /// replaced.
        #[arg(long, conflicts_with_all = ["slice", "split", "follow_up", "comment"])]
        reuse: Option<u64>,
    },
}

#[derive(Debug, Clone, Copy, clap::Args)]
#[group(multiple = false)]
pub struct FileTarget {
    /// Slice n of slices.md, appended to the card's `slices:`.
    #[arg(long)]
    pub slice: Option<u32>,
    /// split-issue.md, appended to the card's `split:`.
    #[arg(long)]
    pub split: bool,
    /// follow-up-k.md.
    #[arg(long)]
    pub follow_up: Option<u32>,
    /// issue-comment.md on the tracking issue, and on the epic of a slice.
    #[arg(long)]
    pub comment: bool,
}

// Four flags of one exclusive group, read once into `request::Part`; lifts
// when clap can map a flag group onto one enum field.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Copy, clap::Args)]
#[group(multiple = false)]
pub struct RequestPart {
    /// An assembled body's spec, architecture, requirements and scenarios.
    #[arg(long)]
    pub spec: bool,
    /// The epic number of a slice issue.
    #[arg(long)]
    pub epic: bool,
    /// A slice issue's whole `**Part of epic**` line.
    #[arg(long)]
    pub slice_line: bool,
    /// The issue's comments.
    #[arg(long)]
    pub comments: bool,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum BodyForm {
    /// Progress, from the first push to gate 4.
    Draft,
    /// At gate 4, from pr.md.
    Full,
}

#[derive(Debug, Subcommand)]
pub enum PrCommand {
    /// Title, body and comments as compact text.
    View { pr: String },
    /// The PR body from the run's artifacts and cards, ending `Closes
    /// #<issue>`; exit 3 when it names a stage or an agent.
    Body { slug: String, form: BodyForm },
    /// Open the run's draft PR from the rendered draft body; record `pr`.
    Open { slug: String },
    /// Push the run's branch: its head must be a stage's verified commit
    /// and pass the gauntlet; logs the pass, updates or opens the draft PR.
    Push {
        #[arg(required_unless_present = "no_run", conflicts_with = "no_run")]
        slug: Option<String>,
        /// Outside a run: push the current branch, never the main one.
        #[arg(long)]
        no_run: bool,
    },
    /// Gate 4 approved: no open threads, no colliding IDs, the full body,
    /// the PR marked ready, the kept follow-ups filed.
    Publish {
        slug: String,
        /// The follow-ups to file, 1,3, or none [default: all].
        #[arg(long)]
        follow_ups: Option<String>,
    },
    /// The PR's inline review threads.
    #[command(subcommand)]
    Feedback(FeedbackCommand),
    /// The error lines of the latest failed CI run on a branch.
    Failed {
        /// [default: the current branch]
        branch: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum FeedbackCommand {
    /// Write the unresolved threads to a file; print one status line.
    Fetch { pr: String, out: PathBuf },
    /// Fetch the threads into the run's pr-feedback.md, log feedback=,
    /// and move the stages owning the commented files back to inprogress.
    Route { slug: String },
    /// Post every filled `reply:` of the file; `fixed` needs the stage and
    /// the pushed sha and resolves the thread.
    Reply {
        pr: String,
        file: PathBuf,
        stage: Option<String>,
        sha: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum HookCommand {
    /// The session-wide `PreToolUse` guard: refuses writes to the files
    /// only ferrule writes, output floods, and the git and `gh` calls
    /// ferrule runs itself, naming the command to run instead.
    Shell,
    /// The session-wide `PreToolUse` rewrite: hands back the Bash command
    /// with any attribution trailer removed.
    Attribution,
    /// `shell` and `attribution` in one call.
    PreToolUse,
    /// The reviewer's `PreToolUse` guard: nothing that writes, except the
    /// run's review.md and review.verdict.md.
    Reviewer,
    /// The `SessionStart` hook: names the runs an interrupted session left.
    SessionStart,
    /// A git hook: `pre-commit` and `pre-push` refuse the main branch,
    /// `commit-msg <file>` strips attribution trailers from the message.
    Git {
        hook: GitHook,
        /// The arguments git passes the hook.
        args: Vec<String>,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum GitHook {
    PreCommit,
    CommitMsg,
    PrePush,
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
    /// reworded after filing, read from the run's cards (the run is the
    /// spec diff's directory name).
    #[arg(long)]
    pub issue_ids: bool,
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
    /// Commit the staged changes for a stage in review: never on the main
    /// branch, the message stripped of attribution trailers, `commit=`
    /// logged on the card.
    Commit {
        /// The stage card; omitted with `--no-run`.
        #[arg(required_unless_present = "no_run", conflicts_with = "no_run")]
        card: Option<String>,
        /// The message; several make paragraphs.
        #[arg(short, long = "message", required = true)]
        message: Vec<String>,
        /// Outside a run: no card, only the branch and trailer rules.
        #[arg(long)]
        no_run: bool,
    },
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
    /// Record and print a run's metrics line; without a slug, every line
    /// recorded.
    Metrics { slug: Option<String> },
    /// Clean up a finished run: worktree and local branch removed, cards
    /// and artifacts archived. Refuses while the worktree holds unpushed
    /// work or CI is not green.
    Cleanup { slug: String },
    /// Recreate a lost board from the run's issue, its draft PR and git;
    /// re-adds the worktree.
    Rebuild { issue: String, pr: String },
    /// Write a run's archive back out as files.
    Export {
        slug: String,
        /// The archive id `run cleanup` printed [default: the latest].
        #[arg(long)]
        archive: Option<i64>,
        /// [default: .ferrule/export/<slug>-<archive>]
        #[arg(long)]
        to: Option<PathBuf>,
    },
    /// Run the gauntlet (`ferrule.toml` `[[gauntlet.step]]`) in the run's
    /// worktree; one status line, the output in the run's gauntlet.log.
    Gauntlet {
        slug: String,
        /// Run even when this tree passed before.
        #[arg(long)]
        force: bool,
    },
    /// The next step after an agent's status line: logs what the outcome
    /// implies and prints `next=`, `run:`, `do:` and `guide:` lines: the topic to read next.
    Next {
        slug: String,
        #[arg(value_enum)]
        at: NextAt,
        /// The agent's status line, as one argument or several.
        #[arg(required = true, allow_hyphen_values = true)]
        status: Vec<String>,
    },
    /// Reconcile a run's cards with git, and with the issue and PR on the
    /// host, before an interrupted run resumes.
    Reconcile {
        slug: String,
        /// Git only: skip the issue, PR and PR-body rows.
        #[arg(long)]
        local: bool,
    },
    /// Start a run: the parent card and its directory; with the issue the
    /// user named, its request, its brief and, for a slice, its epic.
    Init {
        slug: String,
        #[arg(long)]
        issue: Option<String>,
    },
    /// Record an approved gate (gate1, arch, epic, gate2, stage, replan,
    /// delta, gate3, gate4). gate2 also files the issue, adds the worktree,
    /// moves the parent and creates the stage cards.
    Approve {
        gate: String,
        slug: String,
        /// `key=value`: lane, kind=epic (gate1), issue=new|<n>, branch,
        /// stops=s1,s3 (gate2).
        args: Vec<String>,
    },
    /// Start or approve a re-plan.
    Replan {
        slug: String,
        step: ReplanStep,
        /// With approve: stages to stop after beyond the plan's, s1,s3.
        #[arg(long)]
        stops: Option<String>,
    },
    /// Wait for the PR's checks, log ci=, print the next step and, on
    /// red, the failed run's error lines.
    Ci { slug: String },
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

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum NextAt {
    /// Planner, plan reviewer, the spec author's issue answer.
    Plan,
    /// A re-plan's planner and plan reviewer.
    Replan,
    /// A review in scope branch.
    Branch,
    /// The spec author at gate 4.
    Pr,
    /// `ci=<exit code of gh pr checks>`.
    Ci,
}

#[derive(Debug, Subcommand)]
pub enum PromptCommand {
    /// The lines of approval files the orchestrator quotes, never the rest,
    /// then `Confirm:` over all of them.
    Lines {
        #[arg(required = true)]
        files: Vec<PathBuf>,
        /// decisions.md's line count the user last approved.
        #[arg(long, default_value_t = 0)]
        seen: usize,
    },
    /// Open files for the user (the `[viewer]` command, else stdout) and
    /// record them as the run's approval marker.
    Show {
        #[arg(required = true)]
        files: Vec<PathBuf>,
        /// Print only the shown lines, not each file's quoted lines.
        #[arg(long)]
        no_lines: bool,
    },
    /// A stop's whole message from the run's files and cards: approval
    /// (gate1, arch, epic, gate2, stage, replan, gate3, gate4), question
    /// (question, blocked, delta), notice (auto, gate2-notice,
    /// replan-notice) or the `approved` record. Logs the stop.
    Render {
        kind: String,
        slug: String,
        /// `key=value` arguments: size, reason, recommend, issue, stage,
        /// detail, from, pr, where, agent.
        args: Vec<String>,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ReplanStep {
    /// Log `replan from=`, snapshot the plan.
    Start,
    /// Swap the stage cards from `from` on for the new plan's.
    Approve,
}

#[derive(Debug, Args)]
pub struct GuideArgs {
    /// The topic; `--list` names them all.
    #[arg(required_unless_present_any = ["list", "check"])]
    pub topic: Option<String>,
    /// The run: its lane cuts the topic, its card marks steps done and
    /// fills `{slug}`, `{run_dir}` and the rest. Default: the run whose
    /// worktree holds the current directory.
    #[arg(long)]
    pub slug: Option<String>,
    /// The stage the topic is read for: fills `{stage}` and `{card}`.
    #[arg(long)]
    pub stage: Option<String>,
    /// Cut to this lane instead of the card's.
    #[arg(long, value_parser = ["fast", "full"])]
    pub lane: Option<String>,
    /// Every topic, one line each.
    #[arg(long, conflicts_with_all = ["topic", "check"])]
    pub list: bool,
    /// Fail when a topic names a topic that does not exist or carries a
    /// malformed tag.
    #[arg(long, conflicts_with = "topic")]
    pub check: bool,
}

#[derive(Debug, Args)]
pub struct InstallArgs {
    /// The agent host to write adapters for; v1 ships `claude` only.
    #[arg(long, default_value = "claude", value_parser = ["claude"])]
    pub host: String,
    /// Override stack detection for a new `ferrule.toml`.
    #[arg(long, value_parser = ["rust", "node", "go", "python", "none"])]
    pub stack: Option<String>,
    /// Write nothing; fail when the generated files drift from the lock
    /// or from this binary.
    #[arg(long, conflicts_with_all = ["stack", "force"])]
    pub check: bool,
    /// Overwrite files at generated paths that ferrule did not write.
    #[arg(long)]
    pub force: bool,
}
