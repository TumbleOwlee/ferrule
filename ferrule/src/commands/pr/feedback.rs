use std::io::{self, Write};
use std::path::{Path, PathBuf};

use clap::Subcommand;
use ferrule_core::card::board;
use ferrule_core::config::Config;
use ferrule_core::pr::feedback::{self, Kind};
use ferrule_core::pr::publish;
use ferrule_core::project::{self, Layout};

use crate::cli::stage_name;
use crate::commands::card::store;
use crate::commands::exit;
use crate::commands::issue::post_failed;

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
    let fixed_any = replies.iter().any(|r| r.kind == Kind::Fixed);
    let (stage, sha) = match (stage, sha) {
        (Some(st), Some(sh)) => (st, sh),
        _ if fixed_any => {
            eprintln!("pr feedback: a 'reply: fixed' thread needs <stage> <sha> (the pushed head)");
            return Ok(exit::USAGE);
        }
        _ => ("", ""),
    };
    let (mut fixed, mut declined) = (0, 0);
    for r in &replies {
        let body = feedback::reply_text(r, stage, sha);
        if let Err(e) = feedback::post(config.backend.ci, pr, r, &body, &project::root()) {
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

fn route(config: &Config, slug: &str, out: &mut impl Write) -> io::Result<u8> {
    let layout = Layout::new(project::main_checkout());
    let routed = match store(false)? {
        None => None,
        Some(mut s) => {
            let now = board::now();
            Some(
                publish::Run {
                    conn: &mut s.conn,
                    layout: &layout,
                    config,
                    root: &layout.root,
                    slug,
                    now: &now,
                }
                .route_feedback(),
            )
        }
    };
    match routed {
        None | Some(Err(publish::Error::Card)) => {
            writeln!(out, "route=refused reason=card")?;
            Ok(exit::REFUSED)
        }
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
        Some(Err(e)) => post_failed(e, out),
    }
}
