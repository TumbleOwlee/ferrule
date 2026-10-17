use std::io::{self, Write};
use std::path::Path;

use ferrule_core::body::{self, Form};
use ferrule_core::config::Config;
use ferrule_core::feedback::{self, Kind};
use ferrule_core::project::{self, Layout};
use ferrule_core::publish;
use ferrule_core::{board, git, remote};

use super::card::{sql, store};
use super::exit;
use super::issue::{post_failed, print_body};
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
        PrCommand::Open { slug } => open(&config, slug, out),
        PrCommand::Push {
            slug: Some(slug), ..
        } => push(&config, slug, out),
        PrCommand::Push { slug: None, .. } => push_branch(&config, out),
        PrCommand::Publish { slug, follow_ups } => {
            publish_pr(&config, slug, follow_ups.as_deref(), out)
        }
        PrCommand::Feedback(FeedbackCommand::Route { slug }) => route(&config, slug, out),
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

fn open(config: &Config, slug: &str, out: &mut impl Write) -> io::Result<u8> {
    let layout = Layout::new(project::main_checkout());
    let Some(mut s) = store(false)? else {
        writeln!(out, "post=refused reason=card")?;
        return Ok(exit::REFUSED);
    };
    let now = board::now();
    let mut run = publish::Run {
        conn: &mut s.conn,
        layout: &layout,
        config,
        root: &layout.root,
        slug,
        now: &now,
    };
    match run.open_pr() {
        Ok((n, _)) => {
            writeln!(out, "pr={n} state=draft")?;
            Ok(exit::OK)
        }
        Err(e) => post_failed(e, out),
    }
}

fn with_run<T>(
    config: &Config,
    slug: &str,
    f: impl FnOnce(&mut publish::Run) -> T,
) -> io::Result<Option<T>> {
    let layout = Layout::new(project::main_checkout());
    let Some(mut s) = store(false)? else {
        return Ok(None);
    };
    let now = board::now();
    let mut run = publish::Run {
        conn: &mut s.conn,
        layout: &layout,
        config,
        root: &layout.root,
        slug,
        now: &now,
    };
    Ok(Some(f(&mut run)))
}

fn push(config: &Config, slug: &str, out: &mut impl Write) -> io::Result<u8> {
    let layout = Layout::new(project::main_checkout());
    let pushed = match store(false)? {
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
                .push(),
            )
        }
    };
    match pushed {
        None | Some(Err(publish::Error::Card)) => {
            writeln!(out, "push=refused reason=card")?;
            Ok(exit::REFUSED)
        }
        Some(Ok(Ok(p))) => {
            writeln!(out, "{p}")?;
            Ok(exit::OK)
        }
        Some(Ok(Err(r))) => {
            writeln!(out, "{r}")?;
            Ok(exit::REFUSED)
        }
        Some(Err(e)) => post_failed(e, out),
    }
}

fn publish_pr(
    config: &Config,
    slug: &str,
    keep: Option<&str>,
    out: &mut impl Write,
) -> io::Result<u8> {
    let keep: Option<Vec<u32>> =
        keep.map(|k| k.split(',').filter_map(|x| x.trim().parse().ok()).collect());
    match with_run(config, slug, |r| r.publish(keep.as_deref()))? {
        None | Some(Err(publish::Error::Card)) => {
            writeln!(out, "publish=refused reason=card")?;
            Ok(exit::REFUSED)
        }
        Some(Ok(Ok((pr, filed)))) => {
            let list: Vec<String> = filed.iter().map(|n| format!("#{n}")).collect();
            let list = if list.is_empty() {
                "none".to_owned()
            } else {
                list.join(",")
            };
            writeln!(out, "publish=done pr={pr} follow-ups={list}")?;
            Ok(exit::OK)
        }
        Some(Ok(Err(r))) => {
            writeln!(out, "{r}")?;
            Ok(if matches!(r, publish::PublishRefused::StageRef(_)) {
                exit::CONTENT
            } else {
                exit::REFUSED
            })
        }
        Some(Err(e)) => post_failed(e, out),
    }
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

/// `git push -u origin <current branch>`, refusing the main branch.
fn push_branch(config: &Config, out: &mut impl Write) -> io::Result<u8> {
    let here = std::env::current_dir()?;
    let Some(branch) = ferrule_core::git::out(&here, &["symbolic-ref", "--short", "-q", "HEAD"])
    else {
        writeln!(out, "push=refused reason=no-branch")?;
        return Ok(exit::REFUSED);
    };
    if branch == config.main_branch {
        writeln!(out, "push=refused reason=main")?;
        return Ok(exit::REFUSED);
    }
    if !ferrule_core::git::ok(&here, &["push", "-q", "-u", "origin", &branch]) {
        writeln!(out, "push=refused reason=push")?;
        return Ok(exit::REFUSED);
    }
    writeln!(out, "push=done branch={branch}")?;
    Ok(exit::OK)
}
