use ferrule_core::card::board::Slug;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use clap::Subcommand;
use ferrule_core::card::board;
use ferrule_core::config::Config;
use ferrule_core::pr::feedback::{self, AnswerError};
use ferrule_core::project;
use ferrule_core::refusal::{Exit, Refusal};
use ferrule_core::remote::{self, ItemRef};

use crate::cli::stage_name;
use crate::commands::exit;
use crate::commands::issue::post_failed;
use crate::commands::refuse;

const FB_ROUTE_AFTER_LONG_HELP: &str = "\
Output:
  pr=<n> threads=<k> stages=<list>
      The PR's <k> unresolved threads were written to the run's
      pr-feedback.md, keeping the reply and `posted:` lines of a thread
      still open; <list> is the stages moved back to inprogress (`s0,s2`),
      `none` when <k> is 0. Each thread goes to the stage whose `files`
      hold its path, else the last stage. Logs `feedback=<k>` with the
      moves, in one write.
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
  route=refused reason=move stage=s<n> hint=stage s<n> cannot move back to inprogress: <why>
      The board refused moving a stage back; nothing is logged or moved.
  route=refused reason=host|credentials|not-found|ref detail=\"<error>\" hint=<fix>
      The host or tracker failed: `host` it did not answer or answered
      unreadably (auth, network), `credentials` its credentials are
      missing or malformed, `not-found` nothing matched, `ref` the
      reference is no issue or PR (exit 2).
Exit codes: 0 ok, 1 refused, 2 usage, 4 internal error.
Backend: reads threads from the configured code host.
See: ferrule guide show feedback";

const FB_FETCH_AFTER_LONG_HELP: &str = "\
Output:
  pr=<pr> state=<state> draft=<bool> threads=<k> file=<file>
      Wrote the PR's <k> unresolved inline threads to <file>: per thread a
      `## thread <id>` block with `comment:`, `path:`, the comments (each
      body line quoted with `> `) and an empty `reply:` line. <state> is
      lowercase (`open`).
  feedback=refused reason=host|credentials|not-found|unsupported|ref detail=\"<error>\"
      The host could not be read: what it said, and a hint=<fix>.
Exit codes: 0 ok, 1 refused, 2 usage, 4 internal error.
Backend: reads the threads from the configured code host.
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
  reply=refused reason=missing file=<file> hint=fetch the threads first: ferrule pr feedback fetch <pr> <file>
  reply=refused reason=replies count=<n> detail=\"thread <id>: <why>; …\" hint=correct the replies in <file>; nothing was posted
      A reply is neither fixed nor declined, or a declined one has no
      reason as its first bullet.
  reply=refused reason=stage hint=a `reply: fixed` thread needs <stage> <sha>, the pushed head
      exit 2: give the stage and the pushed commit.
  reply=refused reason=host|credentials|not-found|unsupported|ref thread=<id> detail=\"<error>\"
      A post failed part way, after the pr= line; the earlier ones stay
      posted. Ends in hint=<fix>.
Exit codes: 0 ok, 1 refused, 2 usage, 4 internal error.
Backend: posts to the configured code host.
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
        slug: Slug,
    },
    /// Write a PR's unresolved threads to a file.
    #[command(after_long_help = FB_FETCH_AFTER_LONG_HELP)]
    Fetch {
        /// The PR number or URL.
        pr: ItemRef,
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
        pr: ItemRef,
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

fn fetch(config: &Config, pr: &ItemRef, file: &Path, out: &mut impl Write) -> io::Result<u8> {
    let f = match remote::code_host(config, &project::main_checkout()).threads(pr) {
        Ok(f) => f,
        Err(e) => return refuse(out, &e.refusal("feedback")),
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
    pr: &ItemRef,
    file: &Path,
    stage: Option<&str>,
    sha: Option<&str>,
    out: &mut impl Write,
) -> io::Result<u8> {
    let host = remote::code_host(config, &project::main_checkout());
    let stage_sha = stage.zip(sha);
    let line = |a: feedback::Answered| {
        format!(
            "pr={pr} fixed={} declined={} open={}",
            a.fixed, a.declined, a.open
        )
    };
    match feedback::answer(host.as_ref(), pr, file, stage_sha) {
        Err(_) => {
            let r = Refusal::new("reply", "missing")
                .field("file", file.display())
                .hint("fetch the threads first: ferrule pr feedback fetch <pr> <file>");
            refuse(out, &r)
        }
        Ok(Ok(a)) => {
            writeln!(out, "{}", line(a))?;
            Ok(exit::OK)
        }
        Ok(Err(AnswerError::Problems(problems))) => {
            let r = Refusal::new("reply", "replies")
                .field("count", problems.len())
                .detail(problems.join("; "))
                .hint(format!(
                    "correct the replies in {}; nothing was posted",
                    file.display()
                ));
            refuse(out, &r)
        }
        Ok(Err(AnswerError::NeedsStage)) => {
            let r = Refusal::new("reply", "stage")
                .hint("a `reply: fixed` thread needs <stage> <sha>, the pushed head")
                .exit(Exit::Usage);
            refuse(out, &r)
        }
        Ok(Err(AnswerError::Host(a, thread, e))) => {
            writeln!(out, "{}", line(a))?;
            refuse(out, &e.refusal("reply").field("thread", thread))
        }
    }
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
