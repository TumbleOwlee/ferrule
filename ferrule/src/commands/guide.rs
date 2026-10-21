use ferrule_core::card::board::Slug;
use std::io::{self, Write};
use std::path::Path;

use clap::{Args, Subcommand};
use ferrule_core::config::Config;
use ferrule_core::guide::{self, Context, Lane};
use ferrule_core::project::{self, Layout};
use ferrule_core::refusal::Refusal;
use ferrule_core::term::{self, Style};

use super::board::store;
use super::exit;
use super::{internal, refuse};
use crate::cli::stage_name;

/// Shown after `ferrule guide show --help`.
const SHOW_AFTER_LONG_HELP: &str = "\
Output:
  The topic's markdown, cut to the run's lane (a `<!-- lane: … -->` part
  for the other lane is left out), finished steps marked, placeholders such
  as {slug} and {run_dir} filled. With --section, only those sections.
Refusals (stdout, exit 1):
  guide=refused reason=unknown topic=<name> hint=`ferrule guide list` names the topics
                                 No such topic.
  guide=refused reason=unknown-section topic=<name> section=<heading> hint=<fix>
                                 The topic has no section of that heading
                                 (one line per miss). Followed by one
                                 section=<heading> line for each heading the
                                 topic has; pick from those.
Exit codes: 0 ok, 1 refused, 2 usage, 4 internal error.
Example:
  ferrule guide show gate4 --section 'Gate 4 — pull request' --slug my-run";

/// Shown after `ferrule guide list --help`.
const LIST_AFTER_LONG_HELP: &str = "\
Output:
  The topics in `Orchestrator:`, `Agents:` and `Project:` groups, one
  `  <topic>  <summary>` line each.
Exit codes: 0 ok, 4 internal error.";

/// Shown after `ferrule guide check --help`.
const CHECK_AFTER_LONG_HELP: &str = "\
Output:
  guide-check=ok                 Every topic and section a topic names
                                 exists and every tag is well-formed.
Refusals (stdout, exit 1), one line per problem:
  guide-check=fail topic=<name> names-unknown=<topic>[#<heading>]
                                 The topic names a topic or section that does
                                 not exist. Last key hint=fix the name or add
                                 the topic (or hint=add the heading to the
                                 topic or fix the reference).
  guide-check=fail topic=<name> bad-tag=<line>
                                 A malformed tag comment. Last key hint=a tag
                                 is <!-- lane: fast|full --> or <!-- done:
                                 pr=ready -->.
  guide-check=fail topic=<name> no-summary-line
                                 A project topic must start with
                                 `# <name> — <summary>`. Last key hint=start
                                 the file with # <name> — <summary>.
Exit codes: 0 ok, 1 refused, 4 internal error.";

#[derive(Debug, Subcommand)]
pub enum GuideCommand {
    /// Print one topic, cut to the run's lane.
    ///
    /// Its finished steps are marked, and `{slug}`, `{run_dir}` and the
    /// other placeholders are filled from the run's card.
    #[command(after_long_help = SHOW_AFTER_LONG_HELP)]
    Show(ShowArgs),
    /// List the topics.
    #[command(after_long_help = LIST_AFTER_LONG_HELP)]
    List,
    /// Check that every topic and section a topic names exists and every
    /// tag is well-formed.
    #[command(after_long_help = CHECK_AFTER_LONG_HELP)]
    Check,
}

#[derive(Debug, Args)]
pub struct ShowArgs {
    /// The topic, e.g. `gate2` or `implementer`; `ferrule guide list` names
    /// them all.
    pub topic: String,
    /// Print only this section, by heading text ignoring case; repeatable.
    #[arg(long, value_name = "HEADING")]
    pub section: Vec<String>,
    /// The run the topic is read for [default: the run whose worktree holds
    /// the current directory].
    #[arg(long)]
    pub slug: Option<Slug>,
    /// The stage the topic is read for, `s<n>`.
    #[arg(long, value_parser = stage_name)]
    pub stage: Option<String>,
    /// Cut to this lane instead of the run's.
    #[arg(long, value_parser = ["fast", "full"])]
    pub lane: Option<String>,
}

pub fn run(command: &GuideCommand, style: Option<Style>, out: &mut impl Write) -> io::Result<u8> {
    let layout = Layout::new(project::main_checkout());
    let args = match command {
        GuideCommand::Show(args) => args,
        GuideCommand::List => return list(&layout, style, out),
        GuideCommand::Check => return check(&layout, out),
    };
    show(&layout, args, style, out)
}

fn list(layout: &Layout, style: Option<Style>, out: &mut impl Write) -> io::Result<u8> {
    for line in guide::list(&layout.root) {
        match style {
            Some(s) if !line.starts_with(' ') => writeln!(out, "{}", term::strong(&line, s))?,
            _ => writeln!(out, "{line}")?,
        }
    }
    Ok(exit::OK)
}

fn check(layout: &Layout, out: &mut impl Write) -> io::Result<u8> {
    let problems = guide::check(&layout.root);
    for p in &problems {
        writeln!(out, "guide-check=fail {p}")?;
    }
    if problems.is_empty() {
        writeln!(out, "guide-check=ok")?;
        return Ok(exit::OK);
    }
    Ok(exit::REFUSED)
}

fn show(
    layout: &Layout,
    args: &ShowArgs,
    style: Option<Style>,
    out: &mut impl Write,
) -> io::Result<u8> {
    let name = args.topic.as_str();
    let Some(text) = guide::load(&layout.root, name) else {
        let r = Refusal::new("guide", "unknown")
            .field("topic", name)
            .hint("`ferrule guide list` names the topics");
        return refuse(out, &r);
    };
    let ctx = context(layout, args)?;
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
                let r = Refusal::new("guide", "unknown-section")
                    .field("topic", name)
                    .field("section", m)
                    .hint("pick a heading from the section= lines that follow");
                writeln!(out, "{r}")?;
            }
            for h in guide::headings(body) {
                writeln!(out, "section={h}")?;
            }
            Ok(exit::REFUSED)
        }
    }
}

/// The run whose worktree holds the current directory.
fn slug_of_cwd(layout: &Layout) -> Option<Slug> {
    let cwd = std::env::current_dir().ok()?;
    let worktrees = layout.root.join(".ferrule").join("worktrees");
    let rest = cwd.strip_prefix(&worktrees).ok()?;
    rest.components()
        .next()
        .and_then(|c| c.as_os_str().to_str())
        .and_then(|s| s.parse().ok())
}

fn abs(p: &Path) -> String {
    std::path::absolute(p)
        .unwrap_or_else(|_| p.to_path_buf())
        .display()
        .to_string()
}

fn context(layout: &Layout, args: &ShowArgs) -> io::Result<Context> {
    let config = Config::load(&layout.root).map_err(internal)?;
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
            None => slug.to_string(),
        };
        ctx.vars.push(("card", card));
        let held = match store()? {
            Some(s) => s
                .read(|r| match r.state(&slug)? {
                    Some(_) => Ok::<_, ferrule_core::store::Error>(Some((
                        r.fields(&slug)?,
                        r.events(&slug, &[])?,
                    ))),
                    None => Ok(None),
                })
                .map_err(internal)?,
            None => None,
        };
        if let Some((fields, events)) = held {
            for (k, v) in fields {
                match k.as_str() {
                    "lane" => ctx.lane = Lane::parse(&v),
                    "branch" => ctx.vars.push(("branch", v.clone())),
                    "issue" => ctx.vars.push(("issue", v.clone())),
                    "pr" => ctx.vars.push(("pr", v.clone())),
                    _ => {}
                }
                ctx.done.push(format!("{k}={v}"));
            }
            for e in events {
                if let Some(v) = &e.value {
                    ctx.done.push(format!("{}{v}", e.name));
                }
                for (k, v) in &e.args {
                    if let Some(k) = k {
                        ctx.done.push(format!("{k}={v}"));
                    }
                }
            }
        }
        ctx.vars.push(("slug", slug.to_string()));
    }
    if let Some(l) = args.lane.as_deref().and_then(Lane::parse) {
        ctx.lane = Some(l);
    }
    Ok(ctx)
}
