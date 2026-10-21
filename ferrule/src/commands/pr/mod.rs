mod feedback;

use ferrule_core::card::board::Slug;
use std::io::{self, Write};
use std::path::Path;

use clap::{Subcommand, ValueEnum};
use ferrule_core::body::{self, Form};
use ferrule_core::config::Config;
use ferrule_core::git;
use ferrule_core::pr::publish;
use ferrule_core::project::{self, Layout};
use ferrule_core::refusal::Refusal;
use ferrule_core::remote::{self, ItemRef};
use ferrule_core::store::Board;
use ferrule_core::term::Style;

use super::board::{run_on, store};
use super::exit;
use super::issue::{post_failed, print_body};
use super::{internal, refuse};
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
  <body>                         The description, in full
  --- <n> comment(s) (ferrule pr view <ref> --comment <n|a-b>) ---
                                 Then an index of the comments, numbered 1..n
                                 oldest first, one line each:
  <i> [<author> @ <time>] <k> lines: <first line>
                                 Its first line with words, without a leading
                                 `#`, `>` or list marker, the line cut to the
                                 terminal's width (100 columns piped), `…`
                                 ending it when cut. No comments: `--- 0
                                 comment(s) ---`.
Filters (the head line, URL and author always print):
  --body                         Only the body, no index
  --comments                     Every comment in full instead of the index:
                                 `--- <n> comment(s) ---`, then each as
                                 `[#<i> <author> @ <time>]` and its text;
                                 with --body, the body too
  --comment <spec>               Only these comments in full, as --comments
                                 prints them: `3`, `2-4`, `5-`, `-2`;
                                 repeatable and comma-separated (`1,4-5`).
                                 Numbers are the index's, whatever the other
                                 filters keep.
  --last <n>                     The latest n of the comments kept, in full
  --author <login>               The comments by that author, ignoring case,
                                 in full
  Without --comments or --comment, --last and --author print the body too.
  Filters combine: a comment prints when it passes all of them, --last
  counting last. A filtered count reads `--- <k> of <n> comment(s) ---`.
Examples:
  ferrule pr view <n>
  ferrule pr view <n> --comment 2-3
  ferrule pr view <n> --comments --last 3
  ferrule pr view <n> --body
Refusals (stdout, exit 1):
  view=refused reason=comment max=<n> hint=the pull request has <n> comment(s); pick from 1-<n>
                                 --comment names a number the PR lacks (with
                                 no comments: `hint=the pull request has no
                                 comments; drop --comment`). A spec that is
                                 no number or range is a usage error (exit
                                 2).
Errors (stderr, exit 1): `pr view: <the host's error>` for an unknown PR, a
missing credential or a failing `gh`/`curl`.
Exit codes: 0 ok, 1 refused, 2 usage, 4 internal error.
Backend: reads from the configured CI host (`backend` in ferrule.toml).";

const PR_BODY_AFTER_LONG_HELP: &str = "\
Output (stdout): the body, line 1 `# <title>`, ending `Closes #<issue>`; posts
nothing. `draft` is the progress form, `full` the gate 4 form from pr.md.
Refusals:
  body=refused reason=stage-ref stage-ref=<token> hint=reword the source text so it names no stage or agent
      The visible text names a stage (`s<n>`) or an agent (`fe-author`, ...);
      only backticked spans holding `.` or `/` are exempt. Exit 3. Fix: have
      the agent that wrote the source file reword it; never edit the body.
  (no line; stderr `body: missing <path>`)
      A run file the body needs is absent. Exit 1. Fix: run the step that
      writes it.
  (no line, nothing printed)
      The body would be empty. Exit 1.
Exit codes: 0 ok, 1 refused, 3 content (stage-ref), 4 internal error.
See: ferrule guide show gate4";

const PR_OPEN_AFTER_LONG_HELP: &str = "\
Output:
  pr=<n> state=draft
      The run's draft PR <n>; opened now, or the one the card already names.
      Opening records `pr:` on the card and logs `pr=draft`. An open PR of
      the run's branch that the card does not name (one opened before a
      crash) is adopted and its body synced, never opened twice.
Refusals (stdout, exit 1):
  open=refused reason=card hint=check the slug with `ferrule board list`
      No card for the slug.
  open=refused reason=no-branch hint=approve gate2 first: ferrule gate approve <slug> gate2
      The card has no branch yet. Fix: `ferrule gate approve <slug> gate2`.
  open=refused reason=stage-ref stage-ref=<token> hint=reword the source text so it names no stage or agent
      The draft body names a stage or an agent. Exit 3. Fix: see `pr body`.
  open=refused reason=unsupported host=<host> hint=<op> is not implemented for <host> yet; do it on <host> by hand
      The configured CI host cannot open a PR (Bitbucket). Fix: open it on
      the host by hand.
Errors (stderr, exit 1): `body: missing <path>`; the CI host's error.
Exit codes: 0 ok, 1 refused, 3 content, 4 internal error.
Backend: opens the PR on the configured CI host.
See: ferrule guide show stages";

const PR_PUSH_AFTER_LONG_HELP: &str = "\
Output:
  push=done sha=<sha> gauntlet=pass [cached] pr-body=pushed
      Pushed the branch. `cached` when the commit is the reviewed tree. The
      pass is logged as `gauntlet=pass` with cov= and sha= on the card.
      The PR's draft body was updated. `pr-body=unchanged` when it already
      matched.
  push=done sha=<sha> gauntlet=pass pr-body=refused reason=stage-ref stage-ref=<token> hint=reword the text naming it; the PR keeps its previous body
      Pushed (exit 0), but the draft body names a stage or an agent: the
      token sits in a card's `spec-delta` why or a `decisions.md` line.
      Fix: have the agent that wrote it reword it, then push again.
  push=done sha=<sha> gauntlet=pass [cached] pr-body=opened pr=<n>
      The first push: it also opened draft PR <n> and recorded it (or
      adopted the branch's open PR, as `pr open` does).
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
  push=refused reason=dirty hint=commit the changes through `ferrule stage commit`, then push again
      The worktree holds changes beyond HEAD (modified or untracked
      files). Fix: commit them through `ferrule stage commit`, push again.
  push=refused reason=gauntlet step=<name> sha=<sha> [timeout=yes] log=<path> hint=fix the failing step, commit, push again
      The gauntlet failed on the head, as `ci gauntlet` reports it. Fix:
      fix the step, commit, push again.
  push=refused reason=no-worktree|no-steps hint=<fix>
      The gauntlet could not run, as `ci gauntlet` refuses it.
  push=refused reason=push detail=\"<git's error>\" hint=git push to origin failed (network, auth or a rejected update); retry once, then ask the user
      `git push` failed; detail is what git said, and the shell guard
      refuses a raw `git push`. Fix: retry once, then ask the user.
  push=refused reason=no-branch|stage-ref|unsupported ...
      Opening the PR on the first push failed, as `pr open` refuses it.
  push=refused reason=card hint=check the slug with `ferrule board list`
      No card for the slug.
Errors (stderr, exit 1): `missing <path>`; the CI host's error.
Exit codes: 0 ok, 1 refused, 4 internal error.
Backend: pushes to origin; updates the PR on the configured CI host.
See: ferrule guide show stages";

const PR_PUBLISH_AFTER_LONG_HELP: &str = "\
Output:
  publish=done pr=<n> follow-ups=<list>
      The PR carries the full title and body and is ready for review (logs
      `pr=ready`). <list> is the filed follow-up issues, `#12,#13`, or
      `none`; they are logged as `follow-ups=[...]`. Each is logged as
      `posted=follow-up issue=<n> k=<k>` once filed. Rerun after a failure
      (a follow-up the tracker refused), a PR already ready with `pr=ready`
      logged is not edited again, `pr=ready` is not logged twice and a
      follow-up filed before is listed, not filed again.
Refusals (stdout, exit 1 unless noted):
  publish=refused reason=gate4 hint=approve gate4 first: ferrule gate approve <slug> gate4
      The parent card has no `gate4` date. Fix: approve gate 4 first.
  publish=refused reason=threads threads=<n> hint=resolve the open threads: ferrule guide show feedback
      <n> review threads are unresolved. Fix: `ferrule guide show feedback`.
  publish=refused reason=collide ids=<ids> hint=renumber the colliding IDs on the branch, push, publish again
      Another run landed the same fixed IDs on main. Fix: renumber them on
      the branch, push, publish again.
  publish=refused reason=collisions-unknown hint=main could not be fetched or shares no history with the branch; ask the user to fix the remote, then publish again
      The ID check could not run: `git fetch origin <main>` failed or the
      branch has no merge base with it. Publishing refuses rather than skip
      the check. Fix: ask the user.
  publish=refused reason=stage-ref stage-ref=<token> hint=reword the text naming it, then publish again
      The full body names a stage or an agent. Exit 3. Fix: reword the text.
  publish=refused reason=card hint=check the slug with `ferrule board list`
      No card for the slug.
  publish=refused reason=stage-ref stage-ref=<token> hint=reword the source text so it names no stage or agent
      As in `pr body full`. Exit 3.
  publish=refused reason=unsupported host=<host> hint=<op> is not implemented for <host> yet; do it on <host> by hand
      The CI host cannot edit or ready the PR, or the tracker cannot file
      the follow-ups. Fix: do it on the host by hand.
Errors (stderr, exit 1): `body: missing <path>` (pr.md); the host's error.
Exit codes: 0 ok, 1 refused, 3 content, 4 internal error.
Backend: edits and readies the PR on the configured CI host; files the
follow-ups on the tracker.
See: ferrule guide show gate4";

#[derive(Debug, Subcommand)]
pub enum PrCommand {
    /// Print a PR's title, body and comment index as compact text.
    #[command(after_long_help = PR_VIEW_AFTER_LONG_HELP)]
    View {
        /// The PR number or URL.
        pr: ItemRef,
        #[command(flatten)]
        view: super::issue::ViewArgs,
    },
    /// Assemble the PR body from the run's files and cards.
    ///
    /// Ends `Closes #<issue>`. Exits 3 when the body names a stage or an
    /// agent.
    #[command(after_long_help = PR_BODY_AFTER_LONG_HELP)]
    Body {
        /// The run's slug.
        slug: Slug,
        /// Which body to assemble.
        form: BodyForm,
    },
    /// Open the run's draft PR and record its number.
    #[command(after_long_help = PR_OPEN_AFTER_LONG_HELP)]
    Open {
        /// The run's slug.
        slug: Slug,
    },
    /// Push the run's branch and update or open the draft PR.
    ///
    /// The branch head must be a stage's verified commit and pass the
    /// gauntlet; the pass is logged.
    #[command(after_long_help = PR_PUSH_AFTER_LONG_HELP)]
    Push {
        /// The run's slug; omitted with `--no-run`.
        #[arg(required_unless_present = "no_run", conflicts_with = "no_run")]
        slug: Option<Slug>,
        /// Push the current branch outside a run, never the main one.
        #[arg(long)]
        no_run: bool,
    },
    /// Publish the PR once gate 4 is approved.
    ///
    /// Refuses without gate 4's `gate4` date, open threads and colliding
    /// IDs; writes the full body, marks
    /// the PR ready and files the kept follow-ups.
    #[command(after_long_help = PR_PUBLISH_AFTER_LONG_HELP)]
    Publish {
        /// The run's slug.
        slug: Slug,
        /// The follow-ups to file: `1,3`, or `none` [default: all].
        #[arg(long, value_name = "LIST")]
        follow_ups: Option<String>,
    },
    /// Fetch, route and answer the PR's inline review threads.
    #[command(subcommand)]
    Feedback(FeedbackCommand),
}

pub fn run(command: &PrCommand, style: Option<Style>, out: &mut impl Write) -> io::Result<u8> {
    let config = Config::load(&project::main_checkout()).map_err(internal)?;
    match command {
        PrCommand::View { pr, view } => match remote::code_host(&config, &project::root()).view(pr)
        {
            Ok(item) => super::issue::print_item(&item, &view.select(), ("pr", pr), style, out),
            Err(e) => {
                eprintln!("pr view: {e}");
                Ok(exit::REFUSED)
            }
        },
        PrCommand::Body { slug, form } => {
            let layout = Layout::new(project::main_checkout());
            let s = match store(false)? {
                Some(s) => s,
                None => Board::memory().map_err(internal)?,
            };
            let form = match form {
                BodyForm::Draft => Form::Draft,
                BodyForm::Full => Form::Full,
            };
            print_body(
                s.read(|r| body::pr(r, &layout, slug, form, &config.main_branch)),
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
    // A detached HEAD names no branch: `HEAD` would be looked up as one.
    let current = || git::out(dir, &["symbolic-ref", "--short", "-q", "HEAD"]);
    let branch = branch
        .map(str::to_owned)
        .or_else(current)
        .unwrap_or_default();
    if branch.is_empty() && query.run.is_none() {
        eprintln!("ci failed: HEAD is on no branch; name it: ferrule ci failed <branch>");
        return Ok(exit::REFUSED);
    }
    match remote::code_host(config, &root).failed(&branch, query) {
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

#[allow(clippy::redundant_closure_for_method_calls)]
fn open(config: &Config, slug: &str, out: &mut impl Write) -> io::Result<u8> {
    let Some(opened) = with_run(config, slug, |r| r.open_pr())? else {
        return refuse(out, &Refusal::card("open"));
    };
    match opened {
        Ok((n, _)) => {
            writeln!(out, "pr={n} state=draft")?;
            Ok(exit::OK)
        }
        Err(e) => post_failed(e, "open", out),
    }
}

fn with_run<T>(
    config: &Config,
    slug: &str,
    f: impl FnOnce(&mut publish::Run) -> T,
) -> io::Result<Option<T>> {
    Ok(store(false)?.map(|mut s| run_on(&mut s, config, slug, f)))
}

#[allow(clippy::redundant_closure_for_method_calls)]
fn push(config: &Config, slug: &str, out: &mut impl Write) -> io::Result<u8> {
    let pushed = with_run(config, slug, |r| r.push())?;
    match pushed {
        None | Some(Err(publish::Error::Card)) => refuse(out, &Refusal::card("push")),
        Some(Ok(Ok(p))) => {
            writeln!(out, "{p}")?;
            Ok(exit::OK)
        }
        Some(Ok(Err(r))) => refuse(out, &r.refusal()),
        Some(Err(e)) => post_failed(e, "push", out),
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
        None | Some(Err(publish::Error::Card)) => refuse(out, &Refusal::card("publish")),
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
        Some(Ok(Err(r))) => refuse(out, &r.refusal()),
        Some(Err(e)) => post_failed(e, "publish", out),
    }
}

/// `git push -u origin <current branch>`, refusing the main branch.
fn push_branch(config: &Config, out: &mut impl Write) -> io::Result<u8> {
    let here = std::env::current_dir()?;
    let Some(branch) = ferrule_core::git::out(&here, &["symbolic-ref", "--short", "-q", "HEAD"])
    else {
        return refuse(
            out,
            &Refusal::new("push", "no-branch").hint("check out a branch first"),
        );
    };
    if branch == config.main_branch {
        return refuse(out, &publish::PushRefused::Main.refusal());
    }
    if let Err(e) = ferrule_core::git::run(&here, &["push", "-q", "-u", "origin", &branch]) {
        return refuse(out, &publish::PushRefused::Push(e).refusal());
    }
    writeln!(out, "push=done branch={branch}")?;
    Ok(exit::OK)
}
