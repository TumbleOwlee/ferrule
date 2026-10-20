use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use clap::{Args, Subcommand, ValueEnum};
use ferrule_core::card::board;
use ferrule_core::config::Config;
use ferrule_core::project::{self, Layout};
use ferrule_core::prompt::render::{Kind, Viewed};
use ferrule_core::prompt::{self, approval};

use super::board::{sql, store};
use super::exit;
use crate::cli::stage_id;

/// Shown after `ferrule prompt render --help`.
const RENDER_AFTER_LONG_HELP: &str = "\
Output:
  The whole message, line by line, to relay verbatim; compose nothing. By
  kind:
  gate1 arch epic gate2 stage replan gate3 gate4
                       `<title> · <slug> · <files with rev>` header; a
                       `Viewer:` note when the files did not open; the quoted
                       lines (Goal:, Entries:, Plan:, Confirm: ...);
                       `Approving will:`; `Options:`; `Answer: ...`
  question             `Question · <slug> · <where> · <agent> · <n> decisions`,
                       question.md verbatim, `Answer: <1: a · 2: b · ...>`
  blocked              a one-decision question: a) answer, b) re-plan, c) stop
  delta                a question with one decision per `Needs user` item of
                       review.verdict.md
  auto                 `Landed <stage> · ... · next: <s<n>|gate 4>`; no log
  gate2-notice, replan-notice
                       one header line, then the Confirm: block; no log
  approved             approved [spec-rev=<n>] [arch-rev=<n>]
                       [decisions-seen=<n>]
Files the stop shows are opened in the viewer and recorded as the approval
marker (see `ferrule prompt show`).

Flags each kind needs:
  auto                 --stage
  replan-notice        --from
  The others read their flags when given; a missing required flag is a
  usage error.

Logged on the card:
  stop kind=approval gate=<gate>   <gate> is gate1, arch, epic, gate2, replan
                                   or gate3 for that kind
  stop kind=approval gate=gate4    kind gate4
  stop kind=approval gate=delta    kind delta
  stop kind=stage                  stage
  stop kind=question               question
  stop kind=blocked                blocked
  spec-rev=<n>, arch-rev=<n>, decisions-seen=<n>
                                   approved: the revisions the user saw

Refusals:
  prompt render: no card for <slug>
                       exit 2; the run has no card; ferrule run init <slug>
                       or fix the slug
  prompt render: a flag this kind needs is missing; see --help
                       exit 2; add --stage (auto) or --from (replan-notice)
  prompt show: ...     the viewer step refused; see `ferrule prompt show`
Exit codes: 0 ok, 1 refused (a shown file is not readable), 2 usage,
3 content (a shown summary is over its cap).
See: ferrule guide show approval";

/// Shown after `ferrule prompt lines --help`.
const LINES_AFTER_LONG_HELP: &str = "\
Output:
  <line>               a quoted line of each file, in file order, never the
                       rest of the file: Goal:, Scenarios:, Entries:, Plan:,
                       Issue:, PR:, Review:, Needs user: ... by file kind
  Confirm:             then `- <decision or slice>` lines: slices no slice or
                       two slices claim, and the decisions made since --seen
  Confirm: nothing flagged
                       when the files confirm and nothing is flagged
Errors (stderr):
  prompt lines: not readable: <file>
                       exit 1; the other files still print; fix the path
Exit codes: 0 ok, 1 refused (a file not readable), 2 usage.
See: ferrule guide show approval";

/// Shown after `ferrule prompt show --help`.
const SHOW_AFTER_LONG_HELP: &str = "\
Output, one block per file:
  prompt show: opened with <viewer>: <path> [(approval marker: <slug>)]
                       the viewer opened it; then the file's quoted lines
                       unless --no-lines
  prompt show: stdout: <path> [(approval marker: <slug>)]
                       no viewer is configured; the whole file follows
  prompt show: viewer failed: <path> [(approval marker: <slug>)]
                       the viewer exited non-zero; exit 1; open it by hand
The files of a run replace that run's approval marker, so the approval that
follows records exactly these revisions.
Errors (stderr):
  prompt show: not readable: <file>
                       exit 1; nothing is shown; fix the path
  prompt show: refused: <file> is <n> lines / <b> bytes, cap is ...
                       exit 3; only *.summary.md and *.verdict.md are capped
                       (`[thresholds]`); have the agent that wrote it trim
                       it, never show the full file instead
Exit codes: 0 ok, 1 refused, 2 usage, 3 content (summary over its cap).
See: ferrule guide show approval";

#[derive(Debug, Subcommand)]
pub enum PromptCommand {
    /// Print a stop's whole message and log the stop.
    ///
    /// Built from the run's files and cards; opens the files the user
    /// reviews in the viewer.
    #[command(after_long_help = RENDER_AFTER_LONG_HELP)]
    Render(PromptRenderArgs),
    /// Print the lines of approval files the user is shown.
    ///
    /// Ends with `Confirm:` over the decisions made since the last approval.
    #[command(after_long_help = LINES_AFTER_LONG_HELP)]
    Lines {
        /// The approval files.
        #[arg(required = true, value_name = "FILE")]
        files: Vec<PathBuf>,
        /// decisions.md's line count at the last approval.
        #[arg(long, default_value_t = 0)]
        seen: usize,
    },
    /// Open files in the viewer and record them as shown.
    ///
    /// Uses the `[viewer]` command of `ferrule.toml`, else prints to stdout.
    #[command(after_long_help = SHOW_AFTER_LONG_HELP)]
    Show {
        /// The files to show.
        #[arg(required = true, value_name = "FILE")]
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
    /// The stop to render; each flag below names the kinds it applies to.
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

pub fn run(command: &PromptCommand, out: &mut impl Write) -> io::Result<u8> {
    match command {
        PromptCommand::Lines { files, seen } => lines(files, *seen, out),
        PromptCommand::Show { files, no_lines } => show(files, !*no_lines, out),
        PromptCommand::Render(args) => render(args, out),
    }
}

fn lines(files: &[PathBuf], seen: usize, out: &mut impl Write) -> io::Result<u8> {
    let paths: Vec<&Path> = files.iter().map(PathBuf::as_path).collect();
    let (lines, unreadable) = prompt::lines::lines(&paths, seen);
    for l in lines {
        writeln!(out, "{l}")?;
    }
    for u in &unreadable {
        eprintln!("prompt lines: not readable: {u}");
    }
    Ok(if unreadable.is_empty() {
        exit::OK
    } else {
        exit::REFUSED
    })
}

/// How a file was shown, or why not.
pub enum Shown {
    Viewer(String),
    Stdout,
    Failed,
}

/// Runs the viewer on `path` with its output kept out of the caller's.
fn view(viewer: &[String], path: &Path) -> Shown {
    let Some((program, args)) = viewer.split_first() else {
        return Shown::Stdout;
    };
    let ok = Command::new(program)
        .args(args)
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    if ok {
        Shown::Viewer(program.clone())
    } else {
        Shown::Failed
    }
}

/// A file shown, how, and the run whose approval marker it joined.
pub type ShownFile = (PathBuf, Shown, Option<String>);

/// Validates, records and opens `files`; the shown line per file. Refuses
/// everything when one file is unreadable (exit 1) or a summary is over
/// its cap (exit 3).
pub fn open(files: &[PathBuf], err: &mut impl Write) -> io::Result<Result<Vec<ShownFile>, u8>> {
    let layout = Layout::new(project::main_checkout());
    let config = Config::load(&layout.root).map_err(sql)?;
    let mut abs = Vec::new();
    for f in files {
        let Ok(text) = std::fs::read_to_string(f) else {
            writeln!(err, "prompt show: not readable: {}", f.display())?;
            return Ok(Err(exit::REFUSED));
        };
        if let Some((lines, bytes)) = approval::over_cap(f, &text, &config.thresholds) {
            writeln!(
                err,
                "prompt show: refused: {} is {lines} lines / {bytes} bytes, cap is {} lines / {} bytes; have the agent that wrote it trim it, never show the full file instead",
                f.display(),
                config.thresholds.summary_max_lines,
                config.thresholds.summary_max_bytes
            )?;
            return Ok(Err(exit::CONTENT));
        }
        abs.push(f.canonicalize()?);
    }
    let runs = match store(true)? {
        Some(mut s) => approval::record(&mut s.conn, &layout, &abs, &board::now()).map_err(sql)?,
        None => vec![None; abs.len()],
    };
    let viewer = config.viewer.command.unwrap_or_default();
    Ok(Ok(abs
        .into_iter()
        .zip(runs)
        .map(|(path, run)| {
            let shown = view(&viewer, &path);
            (path, shown, run)
        })
        .collect()))
}

fn show(files: &[PathBuf], with_lines: bool, out: &mut impl Write) -> io::Result<u8> {
    let shown = match open(files, &mut io::stderr())? {
        Ok(s) => s,
        Err(code) => return Ok(code),
    };
    let mut code = exit::OK;
    for (path, how, run) in shown {
        let marker = run
            .map(|r| format!(" (approval marker: {r})"))
            .unwrap_or_default();
        match how {
            Shown::Viewer(v) => writeln!(
                out,
                "prompt show: opened with {v}: {}{marker}",
                path.display()
            )?,
            Shown::Failed => {
                writeln!(
                    out,
                    "prompt show: viewer failed: {}{marker}",
                    path.display()
                )?;
                code = exit::REFUSED;
            }
            Shown::Stdout => {
                writeln!(out, "prompt show: stdout: {}{marker}", path.display())?;
                out.write_all(&std::fs::read(&path)?)?;
                continue;
            }
        }
        if with_lines {
            let (lines, _) = prompt::lines::lines(&[path.as_path()], 0);
            for l in lines {
                writeln!(out, "{l}")?;
            }
        }
    }
    Ok(code)
}

fn kind(k: PromptKind) -> Kind {
    match k {
        PromptKind::Gate1 => Kind::Gate1,
        PromptKind::Arch => Kind::Arch,
        PromptKind::Epic => Kind::Epic,
        PromptKind::Gate2 => Kind::Gate2,
        PromptKind::Stage => Kind::Stage,
        PromptKind::Replan => Kind::Replan,
        PromptKind::Gate3 => Kind::Gate3,
        PromptKind::Gate4 => Kind::Gate4,
        PromptKind::Question => Kind::Question,
        PromptKind::Blocked => Kind::Blocked,
        PromptKind::Delta => Kind::Delta,
        PromptKind::Auto => Kind::Auto,
        PromptKind::Gate2Notice => Kind::Gate2Notice,
        PromptKind::ReplanNotice => Kind::ReplanNotice,
        PromptKind::Approved => Kind::Approved,
    }
}

/// The flags as the `(key, value)` pairs `prompt::render` reads.
fn pairs(args: &PromptRenderArgs) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    let stages = [("stage", args.stage), ("from", args.from)];
    for (k, n) in stages {
        if let Some(n) = n {
            pairs.push((k.to_owned(), format!("s{n}")));
        }
    }
    let texts = [
        ("reason", &args.reason),
        ("detail", &args.detail),
        ("recommend", &args.recommend),
        ("issue", &args.issue),
        ("pr", &args.pr),
        ("where", &args.at),
        ("agent", &args.agent),
    ];
    for (k, v) in texts {
        if let Some(v) = v {
            pairs.push((k.to_owned(), v.clone()));
        }
    }
    if args.epic {
        pairs.push(("size".to_owned(), "epic".to_owned()));
    }
    pairs
}

fn render(args: &PromptRenderArgs, out: &mut impl Write) -> io::Result<u8> {
    let slug = args.slug.as_str();
    let kind = kind(args.kind);
    let pairs = pairs(args);
    let Some(mut s) = store(false)? else {
        eprintln!("prompt render: no card for {slug}");
        return Ok(exit::USAGE);
    };
    let layout = Layout::new(project::main_checkout());
    let mut view = |files: &[PathBuf]| -> Result<Viewed, u8> {
        let shown = match open(files, &mut io::stderr()) {
            Ok(Ok(s)) => s,
            Ok(Err(code)) => return Err(code),
            Err(_) => return Ok(Viewed::Failed),
        };
        if shown.iter().any(|(_, how, _)| matches!(how, Shown::Failed)) {
            Ok(Viewed::Failed)
        } else if shown
            .iter()
            .all(|(_, how, _)| matches!(how, Shown::Viewer(_)))
        {
            Ok(Viewed::Opened)
        } else {
            Ok(Viewed::Unavailable(
                shown.into_iter().map(|(p, _, _)| p).collect(),
            ))
        }
    };
    match prompt::render::render(
        &mut s.conn,
        &layout,
        kind,
        slug,
        &pairs,
        &board::now(),
        &mut view,
    ) {
        Ok(lines) => {
            for l in lines {
                writeln!(out, "{l}")?;
            }
            Ok(exit::OK)
        }
        Err(prompt::render::Error::Usage) => {
            eprintln!("prompt render: a flag this kind needs is missing; see --help");
            Ok(exit::USAGE)
        }
        Err(prompt::render::Error::Card) => {
            eprintln!("prompt render: no card for {slug}");
            Ok(exit::USAGE)
        }
        Err(prompt::render::Error::Show(code)) => Ok(code),
        Err(prompt::render::Error::Sql(e)) => Err(sql(e)),
    }
}
