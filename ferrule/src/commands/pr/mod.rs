mod feedback;

use std::io::{self, Write};
use std::path::Path;

use clap::{Subcommand, ValueEnum};
use ferrule_core::body::{self, Form};
use ferrule_core::card::board;
use ferrule_core::config::Config;
use ferrule_core::pr::publish;
use ferrule_core::project::{self, Layout};
use ferrule_core::term::Style;
use ferrule_core::{git, remote};

use super::card::{sql, store};
use super::exit;
use super::issue::{post_failed, print_body};
use feedback::FeedbackCommand;

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
        /// Which body to assemble.
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
    /// Print the error lines of a failed CI run.
    ///
    /// The branch's latest failed run and its first failing job unless
    /// `--run` and `--job` name others. `run ci` prints the same lines when
    /// it waits for a run's checks.
    Failed {
        /// The branch [default: the current branch].
        #[arg(conflicts_with = "run")]
        branch: Option<String>,
        /// A run id (GitHub) or pipeline uuid or number (Bitbucket).
        #[arg(long, value_name = "ID")]
        run: Option<String>,
        /// The failing job (step) whose name contains this, ignoring case.
        #[arg(long, value_name = "NAME")]
        job: Option<String>,
        /// Lines to print after each error line.
        #[arg(long, value_name = "N", default_value_t = 0)]
        context: usize,
    },
}

pub fn run(command: &PrCommand, style: Option<Style>, out: &mut impl Write) -> io::Result<u8> {
    let config = Config::load(&project::main_checkout()).map_err(sql)?;
    match command {
        PrCommand::View { pr } => match remote::pr(config.backend.ci, pr, &project::root()) {
            Ok(item) => {
                super::issue::print_item(&item, style, out)?;
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
                style,
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
        PrCommand::Feedback(command) => feedback::run(&config, command, out),
        PrCommand::Failed {
            branch,
            run,
            job,
            context,
        } => {
            let query = remote::Query {
                run: run.as_deref(),
                job: job.as_deref(),
                context: *context,
            };
            failed(&config, branch.as_deref(), query, out)
        }
    }
}

fn failed(
    config: &Config,
    branch: Option<&str>,
    query: remote::Query,
    out: &mut impl Write,
) -> io::Result<u8> {
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
    match remote::failed(config.backend.ci, &branch, query, &root) {
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
