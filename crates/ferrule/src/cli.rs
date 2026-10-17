use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

/// Swiss-knife CLI for a spec-driven agent workflow.
///
/// Every command prints `key=value` status lines. Exit codes: 0 done, 1
/// refused, 2 usage, 3 content (a body or file that breaks a rule).
#[derive(Debug, Parser)]
#[command(name = "ferrule", version)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
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

#[derive(Debug, Subcommand)]
pub enum SpecCommand {
    /// Print markdown sections by heading.
    ///
    /// A section runs from its heading to the next heading of equal or
    /// shallower depth. Ask for every heading of one file in one call.
    Section(SectionArgs),
    /// List a markdown file's headings.
    Sections(SectionsArgs),
    /// Print spec entries by fixed ID, as `file:line:text`.
    Id(IdArgs),
    /// Print spec entries that mention a keyword.
    Related(RelatedArgs),
    /// List fixed IDs the branch adds that the target ref now has too.
    Collisions(CollisionsArgs),
    /// Render one part of a run's spec-diff.md.
    Render(RenderArgs),
}

#[derive(Debug, Args)]
pub struct SectionArgs {
    /// Match the first heading that starts with each given text.
    #[arg(long)]
    pub prefix: bool,
    /// Heading texts, e.g. `Status`, ignoring case; a leading `## ` also
    /// pins the level.
    #[arg(required = true, num_args = 1..)]
    pub headings: Vec<String>,
    /// The markdown file.
    pub file: PathBuf,
}

#[derive(Debug, Args)]
pub struct SectionsArgs {
    /// Markdown files.
    #[arg(required = true)]
    pub files: Vec<PathBuf>,
}

#[derive(Debug, Args)]
pub struct IdArgs {
    /// Spec directory [default: docs/specs under the project root].
    #[arg(long)]
    pub dir: Option<PathBuf>,
    /// Fixed IDs, requirements and edge cases mixed.
    #[arg(required = true)]
    pub ids: Vec<String>,
}

#[derive(Debug, Args)]
pub struct RelatedArgs {
    /// Keywords, matched as case-insensitive substrings.
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
    /// The run's spec-diff.md.
    pub spec_diff: PathBuf,
    /// The part to render.
    pub part: RenderPart,
    /// Keep only these scenarios and entries, by local or fixed ID,
    /// space-separated.
    #[arg(long, value_name = "IDS")]
    pub only: Option<String>,
    /// With `--only`: also keep "Other spec changes" rows that cite no ID.
    #[arg(long, requires = "only")]
    pub with_unnumbered: bool,
    /// The PR's form of the entries table: an Issue column, and the
    /// requirements reworded after filing.
    ///
    /// Read from the run's cards; the run is the spec diff's directory name.
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
    /// List every card with its state and title.
    Board,
    /// Print a card: frontmatter and log.
    Show {
        /// Card id: `<slug>` or `<slug>.s<n>`.
        card: String,
    },
    /// Print one value of a card; nothing when absent.
    Field {
        /// Card id: `<slug>` or `<slug>.s<n>`.
        card: String,
        /// A frontmatter field, `log:<key>` (the last `<key>=` in the log),
        /// `pass-sha` (the sha of the last `gauntlet=pass`) or `state`.
        field: String,
    },
    /// Create a card.
    New {
        /// Card id: `<slug>` or `<slug>.s<n>`.
        card: String,
        #[arg(long, value_enum, default_value_t = CardState::Open)]
        state: CardState,
        /// Frontmatter fields, `key=value` each.
        fields: Vec<String>,
    },
    /// Set frontmatter fields; an empty value removes one.
    Set {
        /// Card id: `<slug>` or `<slug>.s<n>`.
        card: String,
        /// `key=value` each.
        #[arg(required = true)]
        fields: Vec<String>,
    },
    /// Move a card to another state, if the board allows it.
    Move {
        /// Card id: `<slug>` or `<slug>.s<n>`.
        card: String,
        state: CardState,
    },
    /// Append an event to a card's log.
    Log {
        /// Card id: `<slug>` or `<slug>.s<n>`.
        card: String,
        /// `<name>` or `<name>=<value>`; `card vocab` lists them.
        event: String,
        /// `[positional…] [key=value…] [-- free text]`.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// List the events `card log` accepts.
    Vocab,
    /// Commit the staged changes of a stage in review.
    ///
    /// Refuses the main branch and a branch other than the run's, strips
    /// attribution trailers from the message and logs `commit=` on the card.
    Commit {
        /// The stage card, `<slug>.s<n>`; omitted with `--no-run`.
        #[arg(required_unless_present = "no_run", conflicts_with = "no_run")]
        card: Option<String>,
        /// The message; several make paragraphs.
        #[arg(short, long = "message", required = true)]
        message: Vec<String>,
        /// Commit outside a run: only the branch and trailer rules apply.
        #[arg(long)]
        no_run: bool,
    },
    /// Load a markdown board into the state database: all cards or none.
    Import {
        /// Directory holding `open/`, `inprogress/`, `inreview/` and
        /// `done/`, one `<id>.md` per card.
        board: PathBuf,
    },
}

/// A stage id, `s<n>`.
fn stage_id(s: &str) -> Result<u32, String> {
    s.strip_prefix('s')
        .and_then(|n| n.parse().ok())
        .ok_or_else(|| format!("expected s<n>, got {s}"))
}

#[derive(Debug, Subcommand)]
pub enum RunCommand {
    /// Start a run: its parent card and directory.
    ///
    /// With `--issue`, also the issue's request, its brief and, for a
    /// slice, its epic.
    Init {
        /// The run's slug.
        slug: String,
        /// The issue the user named: a GitHub number or URL, or a Jira key.
        #[arg(long)]
        issue: Option<String>,
    },
    /// Record an approved gate.
    ///
    /// `gate2` also files the issue, adds the worktree, moves the parent
    /// card to inprogress and creates the stage cards.
    Approve(ApproveArgs),
    /// Log an agent's outcome and print the next step.
    ///
    /// Prints `next=`, then `run:` and `do:` lines and a `guide:` line
    /// naming the topic to read next.
    Next {
        /// The run's slug.
        slug: String,
        /// Where the run is.
        #[arg(value_enum)]
        at: NextAt,
        /// The agent's status line, as one argument or several.
        #[arg(required = true, allow_hyphen_values = true)]
        status: Vec<String>,
    },
    /// Run the gauntlet in the run's worktree.
    ///
    /// The steps are `[[gauntlet.step]]` of `ferrule.toml`. Prints one
    /// status line; the output goes to the run's gauntlet.log.
    Gauntlet {
        /// The run's slug.
        slug: String,
        /// Run even when this tree passed before.
        #[arg(long)]
        force: bool,
    },
    /// Verify a stage's reported commit before anything is pushed.
    ///
    /// The commit exists in the worktree, the run's branch holds it, and
    /// it is the stage card's last `commit=`.
    ShaCheck {
        /// The run's slug.
        slug: String,
        /// The stage, `s<n>`.
        #[arg(value_parser = stage_id, value_name = "STAGE")]
        stage: u32,
        /// The commit the implementer reported.
        sha: String,
    },
    /// Decide whether a green full-lane stage stops for approval.
    ///
    /// Logs the outcome on the stage card.
    StageStop {
        /// The run's slug.
        slug: String,
        /// The stage, `s<n>`.
        #[arg(value_parser = stage_id, value_name = "STAGE")]
        stage: u32,
        /// Unresolved PR review threads.
        #[arg(long, default_value_t = 0)]
        threads: u32,
    },
    /// Start or approve a re-plan.
    Replan {
        /// The run's slug.
        slug: String,
        step: ReplanStep,
        /// With `approve`: stages that stop for approval beyond the plan's,
        /// e.g. `s1,s3`.
        #[arg(long)]
        stops: Option<String>,
    },
    /// Wait for the PR's checks and print the next step.
    ///
    /// Logs `ci=`; on red, also prints the failed run's error lines.
    Ci {
        /// The run's slug.
        slug: String,
    },
    /// Reconcile a run's cards with git and the host before resuming.
    Reconcile {
        /// The run's slug.
        slug: String,
        /// Check git only: skip the issue, PR and PR-body rows.
        #[arg(long)]
        local: bool,
    },
    /// Recreate a lost board from the issue, its draft PR and git.
    ///
    /// Also adds the worktree back.
    Rebuild {
        /// The tracking issue.
        issue: String,
        /// The run's draft PR.
        pr: String,
    },
    /// Clean up a finished run.
    ///
    /// Removes the worktree and the local branch and archives the cards and
    /// files. Refuses while the worktree holds unpushed work or CI is not
    /// green.
    Cleanup {
        /// The run's slug.
        slug: String,
    },
    /// Record and print a run's metrics line.
    Metrics {
        /// The run's slug; without one, every line recorded.
        slug: Option<String>,
    },
    /// Write an archived run back out as files.
    Export {
        /// The run's slug.
        slug: String,
        /// The archive id `run cleanup` printed [default: the latest].
        #[arg(long)]
        archive: Option<i64>,
        /// Target directory [default: .ferrule/export/<slug>-<archive>].
        #[arg(long)]
        to: Option<PathBuf>,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum GateName {
    /// The brief.
    Gate1,
    /// The architecture.
    Arch,
    /// An epic's slices.
    Epic,
    /// The plan and the tracking issue.
    Gate2,
    /// A stage stop.
    Stage,
    /// A re-plan.
    Replan,
    /// A meaningful spec change.
    Delta,
    /// The branch review.
    Gate3,
    /// The pull request.
    Gate4,
}

#[derive(Debug, Args)]
pub struct ApproveArgs {
    /// The run's slug.
    pub slug: String,
    pub gate: GateName,
    /// The lane the user settled on.
    #[arg(long, value_parser = ["fast", "full"])]
    pub lane: Option<String>,
    /// gate1: the run is an epic.
    #[arg(long)]
    pub epic: bool,
    /// gate2: `new`, or the number of an issue the run reuses.
    #[arg(long)]
    pub issue: Option<String>,
    /// gate2: the branch [default: feat/<slug>].
    #[arg(long)]
    pub branch: Option<String>,
    /// gate2: stages that stop for approval beyond the plan's, e.g. `s1,s3`.
    #[arg(long)]
    pub stops: Option<String>,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum NextAt {
    /// Planner, plan reviewer, the spec author's issue answer.
    Gate2,
    /// A re-plan's planner and plan reviewer.
    Replan,
    /// The branch review.
    Gate3,
    /// The spec author's PR body.
    Gate4,
    /// `ci=<exit code of gh pr checks>`.
    Ci,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ReplanStep {
    /// Log `replan from=` and snapshot the plan.
    Start,
    /// Swap the stage cards from `from` on for the new plan's.
    Approve,
}

#[derive(Debug, Subcommand)]
pub enum PromptCommand {
    /// Print a stop's whole message and log the stop.
    ///
    /// Built from the run's files and cards; opens the files the user
    /// reviews in the viewer.
    Render(PromptRenderArgs),
    /// Print the lines of approval files the user is shown.
    ///
    /// Ends with `Confirm:` over the decisions made since the last approval.
    Lines {
        /// The approval files.
        #[arg(required = true)]
        files: Vec<PathBuf>,
        /// decisions.md's line count at the last approval.
        #[arg(long, default_value_t = 0)]
        seen: usize,
    },
    /// Open files in the viewer and record them as shown.
    ///
    /// Uses the `[viewer]` command of `ferrule.toml`, else prints to stdout.
    Show {
        /// The files to show.
        #[arg(required = true)]
        files: Vec<PathBuf>,
        /// Print only which files were shown, not their quoted lines.
        #[arg(long)]
        no_lines: bool,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum PromptKind {
    /// Approval: the brief.
    Gate1,
    /// Approval: the architecture.
    Arch,
    /// Approval: an epic's slices.
    Epic,
    /// Approval: the plan and the tracking issue.
    Gate2,
    /// Approval: a stage.
    Stage,
    /// Approval: a re-plan.
    Replan,
    /// Approval: the branch review.
    Gate3,
    /// Approval: the pull request.
    Gate4,
    /// Question: an agent's decision batch.
    Question,
    /// Question: a blocked stage.
    Blocked,
    /// Question: a meaningful spec change.
    Delta,
    /// Notice: a stage continued on its own.
    Auto,
    /// Notice: the fast lane's gate 2.
    Gate2Notice,
    /// Notice: the fast lane's re-plan.
    ReplanNotice,
    /// The record of the last approval.
    Approved,
}

#[derive(Debug, Args)]
pub struct PromptRenderArgs {
    /// The run's slug.
    pub slug: String,
    pub kind: PromptKind,
    /// stage, auto, blocked, delta: the stage, `s<n>`.
    #[arg(long, value_parser = stage_id, value_name = "STAGE")]
    pub stage: Option<u32>,
    /// replan, replan-notice: the first stage re-planned, `s<n>`.
    #[arg(long, value_parser = stage_id, value_name = "STAGE")]
    pub from: Option<u32>,
    /// gate1, stage, blocked: why the run stops.
    #[arg(long)]
    pub reason: Option<String>,
    /// stage: the reason's detail.
    #[arg(long)]
    pub detail: Option<String>,
    /// gate1: the brief proposes an epic.
    #[arg(long)]
    pub epic: bool,
    /// gate1, arch: the recommended lane.
    #[arg(long, value_parser = ["fast", "full"])]
    pub recommend: Option<String>,
    /// gate2, epic: `new`, or the number of an issue the run reuses.
    #[arg(long)]
    pub issue: Option<String>,
    /// gate4: the PR number.
    #[arg(long)]
    pub pr: Option<String>,
    /// question: where the run is, a gate or `s<n>`.
    #[arg(long = "where", value_name = "WHERE")]
    pub at: Option<String>,
    /// question: the agent that asks.
    #[arg(long)]
    pub agent: Option<String>,
}

#[derive(Debug, Subcommand)]
pub enum IssueCommand {
    /// Print an issue's title, body and comments as compact text.
    View {
        /// A GitHub number or URL, or a Jira key.
        issue: String,
    },
    /// Print part of a reused issue, for the run's request.md.
    ///
    /// Without a flag: the issue's original request.
    Request {
        /// A GitHub number or URL, or a Jira key.
        issue: String,
        #[command(flatten)]
        part: RequestPart,
    },
    /// Assemble the tracking issue's body from the run's files.
    ///
    /// Line 1 is `# <title>`. Exits 3 when the body names a stage or an
    /// agent.
    Body {
        /// The run's slug.
        slug: String,
        /// Slice n's own issue instead.
        #[arg(long, value_name = "N")]
        slice: Option<u32>,
    },
    /// File the run's issue, or a related one, and record its number.
    ///
    /// An open issue with the same title is adopted, never filed twice.
    File {
        /// The run's slug.
        slug: String,
        #[command(flatten)]
        target: FileTarget,
        /// Reuse issue n as the tracking issue: its title and body are
        /// replaced.
        #[arg(long, value_name = "N", conflicts_with_all = ["slice", "split", "follow_up", "comment"])]
        reuse: Option<u64>,
    },
}

#[derive(Debug, Clone, Copy, clap::Args)]
#[group(multiple = false)]
pub struct FileTarget {
    /// File slice n of slices.md; appended to the card's `slices:`.
    #[arg(long, value_name = "N")]
    pub slice: Option<u32>,
    /// File split-issue.md; appended to the card's `split:`.
    #[arg(long)]
    pub split: bool,
    /// File follow-up-k.md.
    #[arg(long, value_name = "K")]
    pub follow_up: Option<u32>,
    /// Post issue-comment.md on the tracking issue, and on a slice's epic.
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
    /// Print a PR's title, body and comments as compact text.
    View {
        /// The PR number or URL.
        pr: String,
    },
    /// Assemble the PR body from the run's files and cards.
    ///
    /// Ends `Closes #<issue>`. Exits 3 when the body names a stage or an
    /// agent.
    Body {
        /// The run's slug.
        slug: String,
        form: BodyForm,
    },
    /// Open the run's draft PR and record its number.
    Open {
        /// The run's slug.
        slug: String,
    },
    /// Push the run's branch and update or open the draft PR.
    ///
    /// The branch head must be a stage's verified commit and pass the
    /// gauntlet; the pass is logged.
    Push {
        /// The run's slug; omitted with `--no-run`.
        #[arg(required_unless_present = "no_run", conflicts_with = "no_run")]
        slug: Option<String>,
        /// Push the current branch outside a run, never the main one.
        #[arg(long)]
        no_run: bool,
    },
    /// Publish the PR once gate 4 is approved.
    ///
    /// Refuses open threads and colliding IDs; writes the full body, marks
    /// the PR ready and files the kept follow-ups.
    Publish {
        /// The run's slug.
        slug: String,
        /// The follow-ups to file: `1,3`, or `none` [default: all].
        #[arg(long, value_name = "LIST")]
        follow_ups: Option<String>,
    },
    /// Fetch, route and answer the PR's inline review threads.
    #[command(subcommand)]
    Feedback(FeedbackCommand),
    /// Print the error lines of a branch's latest failed CI run.
    ///
    /// `run ci` prints them too when it waits for a run's checks.
    Failed {
        /// The branch [default: the current branch].
        branch: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum FeedbackCommand {
    /// Fetch a run's threads and send their stages back to work.
    ///
    /// Writes the run's pr-feedback.md, logs `feedback=` and moves the
    /// stages owning the commented files back to inprogress.
    Route {
        /// The run's slug.
        slug: String,
    },
    /// Write a PR's unresolved threads to a file.
    Fetch {
        /// The PR number or URL.
        pr: String,
        /// The file to write.
        out: PathBuf,
    },
    /// Post every filled `reply:` of a feedback file.
    ///
    /// A `fixed` reply needs the stage and the pushed sha, and resolves the
    /// thread.
    Reply {
        /// The PR number or URL.
        pr: String,
        /// The feedback file.
        file: PathBuf,
        /// The stage that fixed the threads, `s<n>`.
        stage: Option<String>,
        /// The pushed commit holding the fixes.
        sha: Option<String>,
    },
}

#[derive(Debug, Args)]
pub struct GuideArgs {
    /// The topic; `--list` names them all.
    #[arg(required_unless_present_any = ["list", "check"])]
    pub topic: Option<String>,
    /// Print only this section, by heading text ignoring case; repeatable.
    #[arg(long, value_name = "HEADING")]
    pub section: Vec<String>,
    /// The run the topic is read for [default: the run whose worktree holds
    /// the current directory].
    #[arg(long)]
    pub slug: Option<String>,
    /// The stage the topic is read for, `s<n>`.
    #[arg(long)]
    pub stage: Option<String>,
    /// Cut to this lane instead of the run's.
    #[arg(long, value_parser = ["fast", "full"])]
    pub lane: Option<String>,
    /// List the topics.
    #[arg(long, conflicts_with_all = ["topic", "check"])]
    pub list: bool,
    /// Check that every topic and section a topic names exists and every
    /// tag is well-formed.
    #[arg(long, conflicts_with = "topic")]
    pub check: bool,
}

#[derive(Debug, Args)]
pub struct InstallArgs {
    /// The agent host to write files for.
    #[arg(long, default_value = "claude", value_parser = ["claude"])]
    pub host: String,
    /// The stack for a new `ferrule.toml` [default: detected].
    #[arg(long, value_parser = ["rust", "node", "go", "python", "none"])]
    pub stack: Option<String>,
    /// Write nothing; fail when the generated files drift from the lock or
    /// from this binary.
    #[arg(long, conflicts_with_all = ["stack", "force"])]
    pub check: bool,
    /// Overwrite files at generated paths that ferrule did not write.
    #[arg(long)]
    pub force: bool,
}

#[derive(Debug, Subcommand)]
pub enum HookCommand {
    /// Guard every Bash call (`PreToolUse`).
    ///
    /// Refuses writes to files only ferrule writes, output floods, and the
    /// git and gh calls a ferrule command replaces, naming that command.
    Shell,
    /// Strip attribution trailers from a Bash call (`PreToolUse`).
    Attribution,
    /// Keep the reviewer read-only (`PreToolUse`).
    ///
    /// Refuses every write except the run's review.md and
    /// review.verdict.md.
    Reviewer,
    /// Name the runs an interrupted session left (`SessionStart`).
    SessionStart,
    /// Run a git hook.
    ///
    /// `pre-commit` and `pre-push` refuse the main branch; `commit-msg`
    /// strips attribution trailers from the message file.
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
