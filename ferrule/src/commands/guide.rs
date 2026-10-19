use std::io::{self, Write};
use std::path::Path;

use clap::Args;
use ferrule_core::card::board;
use ferrule_core::config::Config;
use ferrule_core::guide::{self, Context, Lane};
use ferrule_core::project::{self, Layout};
use ferrule_core::term::{self, Style};

use super::card::{sql, store};
use super::exit;

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

pub fn run(args: &GuideArgs, style: Option<Style>, out: &mut impl Write) -> io::Result<u8> {
    let layout = Layout::new(project::main_checkout());
    if args.list {
        for line in guide::list(&layout.root) {
            match style {
                Some(s) if !line.starts_with(' ') => writeln!(out, "{}", term::strong(&line, s))?,
                _ => writeln!(out, "{line}")?,
            }
        }
        return Ok(exit::OK);
    }
    if args.check {
        let problems = guide::check(&layout.root);
        for p in &problems {
            writeln!(out, "guide-check=fail {p}")?;
        }
        if problems.is_empty() {
            writeln!(out, "guide-check=ok")?;
            return Ok(exit::OK);
        }
        return Ok(exit::REFUSED);
    }
    let name = args.topic.as_deref().unwrap_or_default();
    let Some(text) = guide::load(&layout.root, name) else {
        writeln!(out, "guide=unknown topic={name} list=ferrule guide --list")?;
        return Ok(exit::REFUSED);
    };
    let ctx = context(&layout, args)?;
    let rendered = guide::render(&text, &ctx);
    let body = guide::body(&rendered);
    if args.section.is_empty() {
        super::doc(out, style, body)?;
        return Ok(exit::OK);
    }
    match guide::select(body, &args.section) {
        Ok(text) => {
            super::doc(out, style, &text)?;
            Ok(exit::OK)
        }
        Err(missing) => {
            for m in missing {
                writeln!(out, "guide=unknown-section topic={name} section={m}")?;
            }
            for h in guide::headings(body) {
                writeln!(out, "section={h}")?;
            }
            Ok(exit::REFUSED)
        }
    }
}

/// The run whose worktree holds the current directory.
fn slug_of_cwd(layout: &Layout) -> Option<String> {
    let cwd = std::env::current_dir().ok()?;
    let worktrees = layout.root.join(".ferrule").join("worktrees");
    let rest = cwd.strip_prefix(&worktrees).ok()?;
    rest.components()
        .next()
        .and_then(|c| c.as_os_str().to_str())
        .map(str::to_owned)
}

fn abs(p: &Path) -> String {
    std::path::absolute(p)
        .unwrap_or_else(|_| p.to_path_buf())
        .display()
        .to_string()
}

fn context(layout: &Layout, args: &GuideArgs) -> io::Result<Context> {
    let config = Config::load(&layout.root).map_err(sql)?;
    let mut ctx = Context::default();
    ctx.vars.push(("main", config.main_branch));
    if let Some(stage) = &args.stage {
        ctx.vars.push(("stage", stage.clone()));
    }
    let slug = args.slug.clone().or_else(|| slug_of_cwd(layout));
    if let Some(slug) = slug {
        ctx.vars.push(("run_dir", abs(&layout.run_dir(&slug))));
        ctx.vars.push(("worktree", abs(&layout.worktree(&slug))));
        let card = match &args.stage {
            Some(stage) => format!("{slug}.{stage}"),
            None => slug.clone(),
        };
        ctx.vars.push(("card", card));
        if let Some(s) = store(false)?
            && board::state(&s.conn, &slug).map_err(sql)?.is_some()
        {
            for (k, v) in board::fields(&s.conn, &slug).map_err(sql)? {
                match k.as_str() {
                    "lane" => ctx.lane = Lane::parse(&v),
                    "branch" => ctx.vars.push(("branch", v.clone())),
                    "issue" => ctx.vars.push(("issue", v.clone())),
                    "pr" => ctx.vars.push(("pr", v.clone())),
                    _ => {}
                }
                ctx.done.push(format!("{k}={v}"));
            }
            for (_, body) in board::events(&s.conn, &slug).map_err(sql)? {
                let tokens = body.split(" -- ").next().unwrap_or("");
                ctx.done.extend(
                    tokens
                        .split_whitespace()
                        .filter(|t| t.contains('='))
                        .map(str::to_owned),
                );
            }
        }
        ctx.vars.push(("slug", slug));
    }
    if let Some(l) = args.lane.as_deref().and_then(Lane::parse) {
        ctx.lane = Some(l);
    }
    Ok(ctx)
}
