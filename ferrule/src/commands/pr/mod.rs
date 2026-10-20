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

use super::board::{sql, store};
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

const PR_VIEW_AFTER_LONG_HELP: &str = "\
Output (stdout; compact text, styled on a terminal):
  #<n> <title> [<STATE>]         Line 1: the head (`KEY-1 <summary> [<status>]`
                                 for Jira), then the URL when the host has
                                 one and `by <author>`
  <body>                         The description
  --- <n> comment(s) ---         Then each comment as `[<author> @ <time>]`
                                 and its text
Errors (stderr, exit 1): `pr view: <the host's error>` for an unknown PR, a
missing credential or a failing `gh`/`curl`.
Exit codes: 0 ok, 1 refused.
Backend: reads from the configured CI host (`backend` in ferrule.toml).";

const PR_BODY_AFTER_LONG_HELP: &str = "\
Output (stdout): the body, line 1 `# <title>`, ending `Closes #<issue>`; posts
nothing. `draft` is the progress form, `full` the gate 4 form from pr.md.
Refusals:
  body=refused stage-ref=<token> hint=reword the source text so it names no stage or agent
      The visible text names a stage (`s<n>`) or an agent (`fe-author`, ...);
      only backticked spans holding `.` or `/` are exempt. Exit 3. Fix: have
      the agent that wrote the source file reword it; never edit the body.
  (no line; stderr `body: missing <path>`)
      A run file the body needs is absent. Exit 1. Fix: run the step that
      writes it.
  (no line; stderr `body: no slice <n> in slices.md`)
      `--slice` names a slice slices.md does not have. Exit 1.
  (no line, nothing printed)
      The body would be empty. Exit 1.
Exit codes: 0 ok, 1 refused, 3 content (stage-ref).
See: ferrule guide show gate4";

const PR_OPEN_AFTER_LONG_HELP: &str = "\
Output:
  pr=<n> state=draft
      The run's draft PR <n>; opened now, or the one the card already names.
      Opening records `pr:` on the card and logs `pr=draft`.
Refusals (stdout, exit 1):
  post=refused reason=card hint=no card for this slug; check it with `ferrule board list`
      No card for the slug. Fix: check the slug with `ferrule board list`.
  post=refused reason=no-branch hint=approve gate2 first: ferrule gate approve <slug> gate2
      The card has no branch yet. Fix: `ferrule gate approve <slug> gate2`.
  body=refused stage-ref=<token> hint=reword the source text so it names no stage or agent
      The draft body names a stage or an agent. Exit 3. Fix: see `pr body`.
Errors (stderr, exit 1): `body: missing <path>`; the CI host's error.
Exit codes: 0 ok, 1 refused, 3 content.
Backend: opens the PR on the configured CI host.
See: ferrule guide show stages";

const PR_PUSH_AFTER_LONG_HELP: &str = "\
Output:
  push=done sha=<sha> gauntlet=pass [cached] pr-body=pushed
      Pushed the branch. `cached` when the commit is the reviewed tree. The
      pass is logged as `gauntlet=pass` with cov= and sha= on the card.
      The PR's draft body was updated. `pr-body=unchanged` when it already
      matched.
  push=done sha=<sha> gauntlet=pass pr-body=refused stage-ref=<token> hint=reword the text naming it; the PR keeps its previous body
      Pushed (exit 0), but the draft body names a stage or an agent: the
      token sits in a card's `spec-delta` why or a `decisions.md` line.
      Fix: have the agent that wrote it reword it, then push again.
  push=done sha=<sha> gauntlet=pass [cached] pr-body=opened pr=<n>
      The first push: it also opened draft PR <n> and recorded it.
  push=done branch=<branch>
      `--no-run`: `git push -u origin <branch>`; no gauntlet, no PR.
Refusals (stdout, exit 1):
  push=refused reason=main hint=push from a branch other than main
      The branch is the main branch. Fix: push from another branch.
  push=refused reason=branch hint=approve gate2 first: ferrule gate approve <slug> gate2
      The card has no branch. Fix: `ferrule gate approve <slug> gate2`.
  push=refused reason=no-branch hint=check out a branch first
      `--no-run` on a detached head. Fix: check out a branch.
  push=refused reason=sha mismatch=<why> hint=push a stage's last commit= on the run's branch: ferrule stage verify
      The worktree head is not a stage's last verified `commit=`. <why> is
      card, missing, off-branch or card-commit (see `ferrule stage verify`).
  push=refused reason=gauntlet <gauntlet line> hint=fix the failing step, commit, push again
      The gauntlet failed on the head; the line is `ci gauntlet`'s. Fix: fix
      the step, commit, push again.
  push=refused reason=push hint=git push to origin failed (network, auth or a rejected update); retry once, then ask the user
      `git push` failed; ferrule does not show git's error, and the shell
      guard refuses a raw `git push`. Fix: retry once, then ask the user.
  push=refused reason=card hint=no card for this slug; check it with `ferrule board list`
      No card for the slug. Fix: check the slug with `ferrule board list`.
Errors (stderr, exit 1): `missing <path>`; the CI host's error.
Exit codes: 0 ok, 1 refused.
Backend: pushes to origin; updates the PR on the configured CI host.
See: ferrule guide show stages";

const PR_PUBLISH_AFTER_LONG_HELP: &str = "\
Output:
  publish=done pr=<n> follow-ups=<list>
      The PR carries the full title and body and is ready for review (logs
      `pr=ready`). <list> is the filed follow-up issues, `#12,#13`, or
      `none`; they are logged as `follow-ups=[...]`.
Refusals (stdout, exit 1 unless noted):
  publish=refused reason=threads threads=<n> hint=resolve the open threads: ferrule guide show feedback
      <n> review threads are unresolved. Fix: `ferrule guide show feedback`.
  publish=refused reason=collide ids=<ids> hint=renumber the colliding IDs on the branch, push, publish again
      Another run landed the same fixed IDs on main. Fix: renumber them on
      the branch, push, publish again.
  publish=refused reason=stage-ref stage-ref=<token> hint=reword the text naming it, then publish again
      The full body names a stage or an agent. Exit 3. Fix: reword the text.
  publish=refused reason=card hint=no card for this slug; check it with `ferrule board list`
      No card for the slug. Fix: check the slug with `ferrule board list`.
  body=refused stage-ref=<token> hint=reword the source text so it names no stage or agent
      As in `pr body full`. Exit 3.
Errors (stderr, exit 1): `body: missing <path>` (pr.md); the host's error.
Exit codes: 0 ok, 1 refused, 3 content.
Backend: edits and readies the PR on the configured CI host; files the
follow-ups on the tracker.
See: ferrule guide show gate4";

#[derive(Debug, Subcommand)]
pub enum PrCommand {
    /// Print a PR's title, body and comments as compact text.
    #[command(after_long_help = PR_VIEW_AFTER_LONG_HELP)]
    View {
        /// The PR number or URL.
        pr: String,
    },
    /// Assemble the PR body from the run's files and cards.
    ///
    /// Ends `Closes #<issue>`. Exits 3 when the body names a stage or an
    /// agent.
    #[command(after_long_help = PR_BODY_AFTER_LONG_HELP)]
    Body {
        /// The run's slug.
        slug: String,
        /// Which body to assemble.
        form: BodyForm,
    },
    /// Open the run's draft PR and record its number.
    #[command(after_long_help = PR_OPEN_AFTER_LONG_HELP)]
    Open {
        /// The run's slug.
        slug: String,
    },
    /// Push the run's branch and update or open the draft PR.
    ///
    /// The branch head must be a stage's verified commit and pass the
    /// gauntlet; the pass is logged.
    #[command(after_long_help = PR_PUSH_AFTER_LONG_HELP)]
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
    #[command(after_long_help = PR_PUBLISH_AFTER_LONG_HELP)]
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
    }
}

pub(crate) fn failed(
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
            eprintln!("ci failed: {e}");
            Ok(exit::REFUSED)
        }
    }
}

fn open(config: &Config, slug: &str, out: &mut impl Write) -> io::Result<u8> {
    let layout = Layout::new(project::main_checkout());
    let Some(mut s) = store(false)? else {
        writeln!(
            out,
            "post=refused reason=card hint=no card for this slug; check it with `ferrule board list`"
        )?;
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
            writeln!(
                out,
                "push=refused reason=card hint=no card for this slug; check it with `ferrule board list`"
            )?;
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
            writeln!(
                out,
                "publish=refused reason=card hint=no card for this slug; check it with `ferrule board list`"
            )?;
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
        writeln!(
            out,
            "push=refused reason=no-branch hint=check out a branch first"
        )?;
        return Ok(exit::REFUSED);
    };
    if branch == config.main_branch {
        writeln!(
            out,
            "push=refused reason=main hint=push from a branch other than main"
        )?;
        return Ok(exit::REFUSED);
    }
    if !ferrule_core::git::ok(&here, &["push", "-q", "-u", "origin", &branch]) {
        writeln!(
            out,
            "push=refused reason=push hint=git push to origin failed (network, auth or a rejected update); retry once, then ask the user"
        )?;
        return Ok(exit::REFUSED);
    }
    writeln!(out, "push=done branch={branch}")?;
    Ok(exit::OK)
}
