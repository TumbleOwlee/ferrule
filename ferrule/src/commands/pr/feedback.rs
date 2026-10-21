use std::io::{self, Write};
use std::path::{Path, PathBuf};

use clap::Subcommand;
use ferrule_core::card::board;
use ferrule_core::config::Config;
use ferrule_core::pr::feedback::{self, Kind, Posted};
use ferrule_core::project;
use ferrule_core::refusal::Refusal;

use crate::cli::stage_name;
use crate::commands::exit;
use crate::commands::issue::post_failed;
use crate::commands::refuse;

const FB_ROUTE_AFTER_LONG_HELP: &str = "\
Output:
  pr=<n> threads=<k> stages=<list>
      The PR's <k> unresolved threads were written to the run's
      pr-feedback.md; <list> is the stages moved back to inprogress
      (`s0,s2`), `none` when <k> is 0. Each thread goes to the stage whose
      `files` hold its path, else the last stage. Logs `feedback=<k>`.
  pr-body=<state>
      Printed after it when a stage moved: `pushed` or `unchanged`.
  pr-body=refused reason=stage-ref stage-ref=<token> hint=reword the text naming it; the PR keeps its previous body
      The draft body names a stage or an agent (the token sits in a card's
      `spec-delta` why or a `decisions.md` line); the PR keeps its body.
Refusals (stdout, exit 1):
  route=refused reason=card hint=check the slug with `ferrule board list`
      No card for the slug.
  route=refused reason=no-pr hint=no PR is recorded for this run; open it with `ferrule pr open <slug>`
      The card has no PR.
Errors (stderr, exit 1): the CI host's error.
Exit codes: 0 ok, 1 refused, 4 internal error.
Backend: reads threads from the configured CI host.
See: ferrule guide show feedback";

const FB_FETCH_AFTER_LONG_HELP: &str = "\
Output:
  pr=<pr> state=<state> draft=<bool> threads=<k> file=<file>
      Wrote the PR's <k> unresolved inline threads to <file>: per thread a
      `## thread <id>` block with `comment:`, `path:`, the comments (each
      body line quoted with `> `) and an empty `reply:` line. <state> is
      lowercase (`open`).
Errors (stderr, exit 1): `pr feedback: <the host's error>`.
Exit codes: 0 ok, 1 refused, 4 internal error.
Backend: reads the threads from the configured CI host.
See: ferrule guide show feedback";

const FB_REPLY_AFTER_LONG_HELP: &str = "\
Output:
  pr=<pr> fixed=<n> declined=<n> open=<n>
      Posted every thread whose `reply:` is `fixed` or `declined`, in file
      order. `fixed` resolves the thread; `declined` leaves it open. <open> is
      the file's threads minus the fixed ones. The same line is printed, with
      exit 1, when a post fails part way (the earlier ones stay posted).
      Each step done is written into the file under its reply as
      `posted: replied` (posted) or `posted: resolved` (fixed and resolved);
      a rerun skips what a `posted:` line records and counts it, so nothing
      is posted twice.
Errors (stderr):
  pr feedback: no such file: <file>                 exit 1
  pr feedback: thread <id>: <the host's error>      exit 1
  pr feedback: a 'reply: fixed' thread needs <stage> <sha> (the pushed head)
      exit 2. Fix: give the stage and the pushed commit.
Exit codes: 0 ok, 1 refused, 2 usage, 4 internal error.
Backend: posts to the configured CI host.
See: ferrule guide show feedback";

#[derive(Debug, Subcommand)]
pub enum FeedbackCommand {
    /// Fetch a run's threads and send their stages back to work.
    ///
    /// Writes the run's pr-feedback.md, logs `feedback=` and moves the
    /// stages owning the commented files back to inprogress.
    #[command(after_long_help = FB_ROUTE_AFTER_LONG_HELP)]
    Route {
        /// The run's slug.
        slug: String,
    },
    /// Write a PR's unresolved threads to a file.
    #[command(after_long_help = FB_FETCH_AFTER_LONG_HELP)]
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
    #[command(after_long_help = FB_REPLY_AFTER_LONG_HELP)]
    Reply {
        /// The PR number or URL.
        pr: String,
        /// The feedback file.
        file: PathBuf,
        /// The stage that fixed the threads, `s<n>`; needed for a `fixed`
        /// reply.
        #[arg(value_parser = stage_name, requires = "sha")]
        stage: Option<String>,
        /// The pushed commit holding the fixes; given with the stage.
        sha: Option<String>,
    },
}

pub fn run(config: &Config, command: &FeedbackCommand, out: &mut impl Write) -> io::Result<u8> {
    match command {
        FeedbackCommand::Route { slug } => route(config, slug, out),
        FeedbackCommand::Fetch { pr, out: file } => fetch(config, pr, file, out),
        FeedbackCommand::Reply {
            pr,
            file,
            stage,
            sha,
        } => reply(config, pr, file, stage.as_deref(), sha.as_deref(), out),
    }
}

fn fetch(config: &Config, pr: &str, file: &Path, out: &mut impl Write) -> io::Result<u8> {
    let f = match feedback::fetch(config.backend.ci, pr, &project::root()) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("pr feedback: {e}");
            return Ok(exit::REFUSED);
        }
    };
    std::fs::write(file, f.file(&board::now()))?;
    writeln!(
        out,
        "pr={pr} state={} draft={} threads={} file={}",
        f.state,
        f.draft,
        f.threads.len(),
        file.display()
    )?;
    Ok(exit::OK)
}

fn reply(
    config: &Config,
    pr: &str,
    file: &Path,
    stage: Option<&str>,
    sha: Option<&str>,
    out: &mut impl Write,
) -> io::Result<u8> {
    let Ok(text) = std::fs::read_to_string(file) else {
        eprintln!("pr feedback: no such file: {}", file.display());
        return Ok(exit::REFUSED);
    };
    let (replies, total) = feedback::replies(&text);
    let fixed_any = replies
        .iter()
        .any(|r| r.kind == Kind::Fixed && r.posted.is_none());
    let (stage, sha) = match (stage, sha) {
        (Some(st), Some(sh)) => (st, sh),
        _ if fixed_any => {
            eprintln!("pr feedback: a 'reply: fixed' thread needs <stage> <sha> (the pushed head)");
            return Ok(exit::USAGE);
        }
        _ => ("", ""),
    };
    let (mut fixed, mut declined) = (0, 0);
    let ci = config.backend.ci;
    let root = project::root();
    let mut text = text;
    for r in &replies {
        let mut step = || -> Result<(), ferrule_core::remote::Error> {
            if r.posted.is_none() {
                let body = feedback::reply_text(r, stage, sha);
                feedback::post(ci, pr, r, &body, &root)?;
                text = feedback::mark(&text, &r.thread, Posted::Replied);
                std::fs::write(file, &text)
                    .map_err(|e| ferrule_core::remote::Error::Command(e.to_string()))?;
            }
            if r.kind == Kind::Fixed && r.posted != Some(Posted::Resolved) {
                feedback::resolve(ci, pr, r, &root)?;
                text = feedback::mark(&text, &r.thread, Posted::Resolved);
                std::fs::write(file, &text)
                    .map_err(|e| ferrule_core::remote::Error::Command(e.to_string()))?;
            }
            Ok(())
        };
        if let Err(e) = step() {
            eprintln!("pr feedback: thread {}: {e}", r.thread);
            writeln!(
                out,
                "pr={pr} fixed={fixed} declined={declined} open={}",
                total - fixed
            )?;
            return Ok(exit::REFUSED);
        }
        match r.kind {
            Kind::Fixed => fixed += 1,
            Kind::Declined => declined += 1,
        }
    }
    writeln!(
        out,
        "pr={pr} fixed={fixed} declined={declined} open={}",
        total - fixed
    )?;
    Ok(exit::OK)
}

#[allow(clippy::redundant_closure_for_method_calls)]
fn route(config: &Config, slug: &str, out: &mut impl Write) -> io::Result<u8> {
    let routed = super::with_run(config, slug, |r| r.route_feedback())?;
    match routed {
        None => refuse(out, &Refusal::card("route")),
        Some(Ok((pr, threads, stages, sync))) => {
            let list: Vec<String> = stages.iter().map(|n| format!("s{n}")).collect();
            let list = if list.is_empty() {
                "none".to_owned()
            } else {
                list.join(",")
            };
            writeln!(out, "pr={pr} threads={threads} stages={list}")?;
            if let Some(sync) = sync {
                writeln!(out, "{sync}")?;
            }
            Ok(exit::OK)
        }
        Some(Err(e)) => post_failed(e, "route", out),
    }
}
