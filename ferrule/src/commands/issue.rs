use ferrule_core::card::board::Slug;
use std::io::{self, Write};

use clap::{Args, Subcommand};
use ferrule_core::body;
use ferrule_core::config::Config;
use ferrule_core::issue::request::{self, Part};
use ferrule_core::pr::publish::{self, Target};
use ferrule_core::project::{self, Layout};
use ferrule_core::refusal::Refusal;
use ferrule_core::remote::{self, IssueTracker, ItemRef};
use ferrule_core::term::{self, Style};

use super::board::{run_on, store};
use super::exit;
use super::{internal, refuse};

const ISSUE_SEARCH_AFTER_LONG_HELP: &str = "\
Output:
  #<n> [<STATE>] <title>         One line per issue whose title or body
                                 mentions every keyword, newest first; quote
                                 a keyword with spaces to match it as a
                                 phrase.
No match:
  issue-search=none              Nothing matches (exit 1); try fewer or
                                 other keywords.
Refusals:
  issue-search=too-many max=<m> hint=rerun with more or more specific keywords
                                 More than `search_max` issues ([thresholds]
                                 of ferrule.toml, default 40) match; none is
                                 printed (exit 1). No count=: only max+1 are
                                 fetched.
Errors (stderr, exit 1): `issue search: <the host's error>`; GitHub only yet.
Exit codes: 0 ok, 1 none or refused, 4 internal error.
Example:
  ferrule issue search --state open modbus timeout
See: ferrule guide show issue";

const ISSUE_VIEW_AFTER_LONG_HELP: &str = "\
Output (stdout; compact text, styled on a terminal):
  #<n> <title> [<STATE>]         Line 1: the head (`KEY-1 <summary> [<status>]`
                                 for Jira), then the URL when the host has
                                 one and `by <author>`
  <body>                         The description, in full
  --- <n> comment(s) (ferrule issue view <ref> --comment <n|a-b>) ---
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
  ferrule issue view <n>
  ferrule issue view <n> --comment 2-3
  ferrule issue view <n> --comments --last 3
  ferrule issue view <n> --body
Refusals (stdout, exit 1):
  issue=refused reason=pull-request hint=ferrule pr view <n>
                                 The number names a pull request (GitHub
                                 serves PRs as issues); view it with pr view.
  view=refused reason=comment max=<n> hint=the issue has <n> comment(s); pick from 1-<n>
                                 --comment names a number the issue lacks
                                 (with no comments: `hint=the issue has no
                                 comments; drop --comment`). A spec that is
                                 no number or range is a usage error (exit
                                 2).
Errors (stderr, exit 1): `issue view: <the host's error>` for an unknown issue, a
missing credential or a failing `gh`/`curl`.
Exit codes: 0 ok, 1 refused, 2 usage, 4 internal error.
Backend: reads from the configured tracker (`backend` in ferrule.toml).";

const ISSUE_REQUEST_AFTER_LONG_HELP: &str = "\
Output (stdout): the requested part as plain text, nothing when the issue has
no such part. With `--comments`, every comment as `[<author> @ <time>]` then
its text.
Errors (stderr, exit 1): `issue request: <the tracker's error>` for an unknown
issue, a missing credential or a failing `gh`/`curl`.
Exit codes: 0 ok, 1 refused, 4 internal error.
Backend: reads from the configured tracker.
See: ferrule guide show epic";

const ISSUE_BODY_AFTER_LONG_HELP: &str = "\
Output (stdout): the body, line 1 `# <title>`, nothing else. Reads the run's
files and cards; posts nothing.
Refusals:
  body=refused reason=stage-ref stage-ref=<token> hint=reword the source text so it names no stage or agent
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
Exit codes: 0 ok, 1 refused, 3 content (stage-ref), 4 internal error.
See: ferrule guide show issue";

const ISSUE_POST_AFTER_LONG_HELP: &str = "\
Output:
  issue=<n> action=<action>
      Filed or updated issue <n>; the card's `issue:` (or `slices:` /
      `split:`) records it. `--slice <n>` records it at position n of
      `slices:` (`[71, -, 73]`, `-` for a slice not filed yet), whatever
      order the slices are filed in. <action> is one of:
        created    A new issue was filed
        adopted    An open issue titled exactly the same became the run's
                   (searched as one quoted phrase, among the first 100 hits)
        edited     The tracking issue's title and body were replaced (an
                   epic's, or `--reuse`)
        commented  `--comment` posted issue-comment.md
        recorded   Nothing was posted: the card already names the issue,
                   the follow-up was filed before, or every target already
                   carries this issue-comment.md
      Each host write that can be retried is logged on the card, so a rerun
      after a failure does only what is missing:
        posted=comment issue=<n> sum=<hash>
                   `--comment` posted this text (by its hash) to issue <n>:
                   the tracking issue, then a slice's epic
        posted=follow-up issue=<n> k=<k>
                   `--follow-up <k>` (or `pr publish`) filed follow-up-k.md
                   as issue <n>
Refusals (stdout, exit 1):
  post=refused reason=card hint=check the slug with `ferrule board list`
      No card for the slug.
  post=refused reason=no-issue hint=file the tracking issue first: ferrule issue post <slug>
      `--split`, `--follow-up` or `--comment` before the card has an issue.
      Fix: `ferrule issue post <slug>` first.
  post=refused reason=stage-ref stage-ref=<token> hint=reword the source text so it names no stage or agent
      The tracking or slice body names a stage or an agent. Exit 3. Fix:
      reword the source file (see `issue body`).
  post=refused reason=unsupported host=<host> hint=<op> is not implemented for <host> yet; do it on <host> by hand
      The configured tracker cannot file, search, edit or comment on
      issues (Jira). Fix: do it on the tracker by hand.
Errors (stderr, exit 1): `missing <path>` for an absent run file;
`body: no slice <n> in slices.md` for a slice slices.md lacks (or 0); the
tracker's error when the host call fails.
Exit codes: 0 ok, 1 refused, 3 content, 4 internal error.
Backend: writes are GitHub-only (`gh`); with another tracker configured the
command refuses (reason=unsupported).
See: ferrule guide show issue";

/// What `issue view` and `pr view` print; without a flag, the body and an
/// index of the comments.
#[derive(Debug, Clone, Args)]
pub struct ViewArgs {
    /// Print only the body; with `--comments` or `--comment`, both. The
    /// head line, URL and author always print.
    #[arg(long)]
    pub body: bool,
    /// Print every comment in full.
    #[arg(long)]
    pub comments: bool,
    /// Print these comments in full: `3`, `2-4`, `5-`, `-2`; repeatable
    /// and comma-separated.
    #[arg(
        long,
        value_name = "N|A-B",
        value_delimiter = ',',
        allow_hyphen_values = true
    )]
    pub comment: Vec<remote::CommentRange>,
    /// Print the latest n of the chosen comments in full.
    #[arg(long, value_name = "N")]
    pub last: Option<usize>,
    /// Print the comments by this author in full, ignoring case.
    #[arg(long, value_name = "LOGIN")]
    pub author: Option<String>,
}

impl ViewArgs {
    pub fn select(&self) -> remote::Select {
        remote::Select {
            body: self.body,
            comments: self.comments,
            last: self.last,
            author: self.author.clone(),
            pick: self.comment.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum SearchState {
    Open,
    Closed,
    All,
}

#[derive(Debug, Subcommand)]
pub enum IssueCommand {
    /// List issues whose title or body mention every keyword.
    #[command(after_long_help = ISSUE_SEARCH_AFTER_LONG_HELP)]
    Search {
        /// Keywords; an issue must mention every one.
        #[arg(required = true, value_name = "KEYWORD")]
        keywords: Vec<String>,
        /// Which issues to search.
        #[arg(long, value_enum, default_value_t = SearchState::All)]
        state: SearchState,
    },
    /// Print an issue's title, body and comment index as compact text.
    #[command(after_long_help = ISSUE_VIEW_AFTER_LONG_HELP)]
    View {
        /// A GitHub number or URL, or a Jira key.
        issue: ItemRef,
        #[command(flatten)]
        view: ViewArgs,
    },
    /// Print part of a reused issue, for the run's request.md.
    ///
    /// Without a flag: the issue's original request.
    #[command(after_long_help = ISSUE_REQUEST_AFTER_LONG_HELP)]
    Request {
        /// A GitHub number or URL, or a Jira key.
        issue: ItemRef,
        #[command(flatten)]
        part: RequestPart,
    },
    /// Assemble the tracking issue's body from the run's files.
    ///
    /// Line 1 is `# <title>`. Exits 3 when the body names a stage or an
    /// agent.
    #[command(after_long_help = ISSUE_BODY_AFTER_LONG_HELP)]
    Body {
        /// The run's slug.
        slug: Slug,
        /// Slice n's own issue instead.
        #[arg(long, value_name = "N")]
        slice: Option<u32>,
    },
    /// Post the run's issue, a related one or a comment; record its number.
    ///
    /// An open issue with the same title is adopted, never filed twice.
    #[command(after_long_help = ISSUE_POST_AFTER_LONG_HELP)]
    Post {
        /// The run's slug.
        slug: Slug,
        #[command(flatten)]
        target: FileTarget,
        /// Reuse this issue (a number or tracker key) as the tracking
        /// issue: its title and body are replaced.
        #[arg(long, value_name = "ISSUE", conflicts_with_all = ["slice", "split", "follow_up", "comment"])]
        reuse: Option<ItemRef>,
    },
}

#[derive(Debug, Clone, Copy, clap::Args)]
#[group(multiple = false)]
pub struct FileTarget {
    /// File slice n of slices.md (from 1); recorded at position n of the
    /// card's `slices:`.
    #[arg(long, value_name = "N")]
    pub slice: Option<u32>,
    /// File split-issue.md; appended to the card's `split:`.
    #[arg(long)]
    pub split: bool,
    /// File follow-up-k.md.
    #[arg(long, value_name = "K")]
    pub follow_up: Option<u32>,
    /// Post issue-comment.md on the tracking issue, and on a slice's epic.
    #[arg(long)]
    pub comment: bool,
}

// Four flags of one exclusive group, read once into `request::Part`; lifts
// when clap can map a flag group onto one enum field.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Copy, clap::Args)]
#[group(multiple = false)]
pub struct RequestPart {
    /// An assembled body's spec, architecture, requirements and scenarios.
    #[arg(long)]
    pub spec: bool,
    /// The epic number of a slice issue.
    #[arg(long)]
    pub epic: bool,
    /// A slice issue's whole `**Part of epic**` line.
    #[arg(long)]
    pub slice_line: bool,
    /// The issue's comments.
    #[arg(long)]
    pub comments: bool,
}

pub fn run(command: &IssueCommand, style: Option<Style>, out: &mut impl Write) -> io::Result<u8> {
    match command {
        IssueCommand::Search { keywords, state } => search(keywords, *state, out),
        IssueCommand::View { issue, view: v } => view(issue, v, style, out),
        IssueCommand::Request { issue, part } => request(issue, *part, style, out),
        IssueCommand::Body { slug, slice } => body(slug, *slice, style, out),
        IssueCommand::Post {
            slug,
            target,
            reuse,
        } => file(slug, *target, reuse.clone(), out),
    }
}

fn search(keywords: &[String], state: SearchState, out: &mut impl Write) -> io::Result<u8> {
    let config = config()?;
    let max = config.thresholds.search_max;
    let state = match state {
        SearchState::Open => remote::SearchState::Open,
        SearchState::Closed => remote::SearchState::Closed,
        SearchState::All => remote::SearchState::All,
    };
    match remote::issue_tracker(&config, &project::main_checkout()).search(keywords, state, max + 1)
    {
        Ok(found) if found.len() > max => {
            writeln!(
                out,
                "issue-search=too-many max={max} hint=rerun with more or more specific keywords"
            )?;
            Ok(exit::REFUSED)
        }
        Ok(found) if found.is_empty() => {
            writeln!(out, "issue-search=none")?;
            Ok(exit::REFUSED)
        }
        Ok(found) => {
            for f in found {
                writeln!(out, "#{} [{}] {}", f.number, f.state, f.title)?;
            }
            Ok(exit::OK)
        }
        Err(e) => {
            eprintln!("issue search: {e}");
            Ok(exit::REFUSED)
        }
    }
}

fn config() -> io::Result<Config> {
    Config::load(&project::main_checkout()).map_err(internal)
}

/// The configured issue tracker, in the current checkout.
fn tracker() -> io::Result<Box<dyn IssueTracker>> {
    Ok(remote::issue_tracker(&config()?, &project::main_checkout()))
}

/// Columns of a comment index line when stdout is no terminal.
const PIPED_WIDTH: usize = 100;

/// Prints an issue or PR (`kind` `issue` or `pr`, named `at`): compact
/// text, or styled with its body and comments rendered; or the refusal of
/// a `--comment` it lacks. Its exit code.
pub fn print_item(
    item: &remote::Item,
    select: &remote::Select,
    (kind, at): (&str, &ItemRef),
    style: Option<Style>,
    out: &mut impl Write,
) -> io::Result<u8> {
    let view = format!("ferrule {kind} view {at}");
    let width = style.map_or(PIPED_WIDTH, |s| s.width);
    let full = if select.full() {
        match select.comments(item) {
            Ok(kept) => Some(kept),
            Err(remote::NoSuchComment { max }) => {
                let noun = if kind == "pr" {
                    "pull request"
                } else {
                    "issue"
                };
                let fix = if max == 0 {
                    format!("the {noun} has no comments; drop --comment")
                } else {
                    format!("the {noun} has {max} comment(s); pick from 1-{max}")
                };
                return refuse(
                    out,
                    &Refusal::new("view", "comment").field("max", max).hint(fix),
                );
            }
        }
    } else {
        None
    };
    let Some(s) = style else {
        let text = item
            .text_for(select, &view, width)
            .map_err(|_| internal("comments checked above"))?;
        write!(out, "{text}")?;
        return Ok(exit::OK);
    };
    writeln!(out, "{}", term::strong(&item.head, s))?;
    let by = match &item.url {
        Some(url) => format!("{url} · by {}", item.by),
        None => format!("by {}", item.by),
    };
    writeln!(out, "{}\n", term::faint(&by, s))?;
    if select.shows_body() {
        write!(out, "{}", term::markdown(&item.body, s))?;
    }
    if select.shows_index() {
        let title = format!("── {} ──", item.index_title(&view));
        writeln!(out, "\n{}", term::faint(&title, s))?;
        for l in item.index(width) {
            writeln!(out, "{l}")?;
        }
    }
    let Some(kept) = full else {
        return Ok(exit::OK);
    };
    let title = format!("── {} ──", item.full_title(kept.len()));
    writeln!(out, "\n{}", term::faint(&title, s))?;
    for (n, c) in kept {
        let head = format!("#{n} {} · {}", c.author, c.at);
        writeln!(out, "\n{}", term::strong(&head, s))?;
        write!(out, "{}", term::markdown(&c.body, s))?;
    }
    Ok(exit::OK)
}

fn view(
    issue: &ItemRef,
    view: &ViewArgs,
    style: Option<Style>,
    out: &mut impl Write,
) -> io::Result<u8> {
    match tracker()?.view(issue) {
        Ok(item) => print_item(&item, &view.select(), ("issue", issue), style, out),
        Err(remote::Error::PullRequest(n)) => refuse(
            out,
            &Refusal::new("issue", "pull-request").hint(format!("ferrule pr view {n}")),
        ),
        Err(e) => {
            eprintln!("issue view: {e}");
            Ok(exit::REFUSED)
        }
    }
}

fn request(
    issue: &ItemRef,
    part: RequestPart,
    style: Option<Style>,
    out: &mut impl Write,
) -> io::Result<u8> {
    let item = match tracker()?.view(issue) {
        Ok(item) => item,
        Err(e) => {
            eprintln!("issue request: {e}");
            return Ok(exit::REFUSED);
        }
    };
    if part.comments {
        let text = item.text();
        let after = text
            .split_once(" comment(s) ---\n")
            .map_or("", |(_, rest)| rest);
        super::doc(out, style, after.strip_prefix('\n').unwrap_or(after))?;
        return Ok(exit::OK);
    }
    let which = if part.spec {
        Part::Spec
    } else if part.epic {
        Part::Epic
    } else if part.slice_line {
        Part::SliceLine
    } else {
        Part::Request
    };
    let mut text = request::part(&item.body, which).join("\n");
    if !text.is_empty() {
        text.push('\n');
    }
    super::doc(out, style, &text)?;
    Ok(exit::OK)
}

/// Prints an assembled body, or the refusal it ran into.
pub fn print_body(
    result: Result<String, body::Error>,
    style: Option<Style>,
    out: &mut impl Write,
) -> io::Result<u8> {
    match result {
        Ok(text) => {
            super::doc(out, style, &text)?;
            Ok(exit::OK)
        }
        Err(e) if let Some(r) = e.refusal("body") => refuse(out, &r),
        Err(body::Error::Missing(p)) => {
            eprintln!("body: missing {}", p.display());
            Ok(exit::REFUSED)
        }
        Err(body::Error::NoSlice(n)) => {
            eprintln!("body: no slice {n} in slices.md");
            Ok(exit::REFUSED)
        }
        Err(body::Error::Empty | body::Error::StageRef(_)) => Ok(exit::REFUSED),
        Err(body::Error::Sql(e)) => Err(internal(e)),
    }
}

fn body(
    slug: &str,
    slice: Option<u32>,
    style: Option<Style>,
    out: &mut impl Write,
) -> io::Result<u8> {
    let layout = Layout::new(project::main_checkout());
    let s = match store(false)? {
        Some(s) => s,
        None => ferrule_core::store::Board::memory().map_err(internal)?,
    };
    print_body(s.read(|r| body::issue(r, &layout, slug, slice)), style, out)
}

/// Prints the `<verb>=refused` line of a failed post, or what went wrong
/// on stderr; its exit code.
pub fn post_failed(e: publish::Error, verb: &'static str, out: &mut impl Write) -> io::Result<u8> {
    if let Some(r) = e.refusal(verb) {
        return refuse(out, &r);
    }
    match e {
        publish::Error::Body(b) => print_body(Err(b), None, out),
        publish::Error::Missing(p) => {
            eprintln!("missing {}", p.display());
            Ok(exit::REFUSED)
        }
        publish::Error::NoIssue
        | publish::Error::NoBranch
        | publish::Error::NoPr
        | publish::Error::Card
        | publish::Error::Moved(..) => {
            unreachable!("refusal() answers these")
        }
        publish::Error::Remote(e) => {
            eprintln!("{e}");
            Ok(exit::REFUSED)
        }
        publish::Error::Sql(e) => Err(internal(e)),
    }
}

fn file(slug: &str, t: FileTarget, reuse: Option<ItemRef>, out: &mut impl Write) -> io::Result<u8> {
    let target = match (t.slice, t.split, t.follow_up, t.comment) {
        (Some(n), ..) => Target::Slice(n),
        (_, true, ..) => Target::Split,
        (_, _, Some(k), _) => Target::FollowUp(k),
        (.., true) => Target::Comment,
        _ => Target::Run,
    };
    let config = config()?;
    let Some(mut s) = store(false)? else {
        return refuse(out, &Refusal::card("post"));
    };
    match run_on(&mut s, &config, slug, |r| r.file(target, reuse)) {
        Ok((n, action)) => {
            writeln!(out, "issue={n} action={}", action.as_str())?;
            Ok(exit::OK)
        }
        Err(e) => post_failed(e, "post", out),
    }
}
