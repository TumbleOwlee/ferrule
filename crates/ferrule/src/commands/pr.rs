use std::io::{self, Write};
use std::path::Path;

use ferrule_core::body::{self, Form};
use ferrule_core::config::Config;
use ferrule_core::feedback::{self, Kind};
use ferrule_core::project::{self, Layout};
use ferrule_core::{board, git, remote};

use super::card::{sql, store};
use super::exit;
use super::issue::print_body;
use crate::cli::{BodyForm, FeedbackCommand, PrCommand};

pub fn run(command: &PrCommand, out: &mut impl Write) -> io::Result<u8> {
    let config = Config::load(&project::main_checkout()).map_err(sql)?;
    match command {
        PrCommand::View { pr } => match remote::pr(config.backend.ci, pr, &project::root()) {
            Ok(item) => {
                write!(out, "{}", item.text())?;
                Ok(exit::OK)
            }
            Err(e) => {
                eprintln!("pr view: {e}");
                Ok(exit::REFUSED)
            }
        },
        PrCommand::Body { slug, form } => {
            let layout = Layout::new(project::main_checkout());
            let s = match store(false)? {
                Some(s) => s,
                None => ferrule_core::store::Store::memory().map_err(sql)?,
            };
            let form = match form {
                BodyForm::Draft => Form::Draft,
                BodyForm::Full => Form::Full,
            };
            print_body(
                body::pr(&s.conn, &layout, slug, form, &config.main_branch),
                out,
            )
        }
        PrCommand::Feedback(FeedbackCommand::Fetch { pr, out: file }) => {
            fetch(&config, pr, file, out)
        }
        PrCommand::Feedback(FeedbackCommand::Reply {
            pr,
            file,
            stage,
            sha,
        }) => reply(&config, pr, file, stage.as_deref(), sha.as_deref(), out),
        PrCommand::Failed { branch } => failed(&config, branch.as_deref(), out),
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

fn failed(config: &Config, branch: Option<&str>, out: &mut impl Write) -> io::Result<u8> {
    let root = project::root();
    let dir = if root.as_os_str().is_empty() {
        Path::new(".")
    } else {
        root.as_path()
    };
    let branch = branch.map_or_else(
        || git::out(dir, &["rev-parse", "--abbrev-ref", "HEAD"]).unwrap_or_default(),
        str::to_owned,
    );
    match remote::failed(config.backend.ci, &branch, &root) {
        Ok(lines) => {
            for l in lines {
                writeln!(out, "{l}")?;
            }
            Ok(exit::OK)
        }
        Err(e) => {
            eprintln!("pr failed: {e}");
            Ok(exit::REFUSED)
        }
    }
}
