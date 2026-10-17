use std::io::{self, Write};
use std::path::Path;

use ferrule_core::board;
use ferrule_core::config::Config;
use ferrule_core::guide::{self, Context, Lane};
use ferrule_core::project::{self, Layout};

use super::card::{sql, store};
use super::exit;
use crate::cli::GuideArgs;

pub fn run(args: &GuideArgs, out: &mut impl Write) -> io::Result<u8> {
    let layout = Layout::new(project::main_checkout());
    if args.list {
        for line in guide::list(&layout.root) {
            writeln!(out, "{line}")?;
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
    write!(out, "{}", guide::render(&text, &ctx))?;
    Ok(exit::OK)
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
