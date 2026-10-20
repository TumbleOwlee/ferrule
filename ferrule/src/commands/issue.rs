use std::io::{self, Write};

use clap::{Args, Subcommand};
use ferrule_core::body;
use ferrule_core::card::board;
use ferrule_core::config::Config;
use ferrule_core::issue::request::{self, Part};
use ferrule_core::pr::publish::{self, Target};
use ferrule_core::project::{self, Layout};
use ferrule_core::remote;
use ferrule_core::term::{self, Style};

use super::board::{sql, store};
use super::exit;

const ISSUE_SEARCH_AFTER_LONG_HELP: &str = "\
Output:
  #<n> [<STATE>] <title>         One line per issue whose title or body
                                 mentions every keyword, newest first.
No match:
  issue-search=none              Nothing matches (exit 1); try fewer or
                                 other keywords.
Refusals:
  issue-search=too-many max=<m> hint=rerun with more or more specific keywords
                                 More than `search_max` issues ([thresholds]
                                 of ferrule.toml, default 40) match; none is
                                 printed (exit 1).
Errors (stderr, exit 1): `issue search: <the host's error>`; GitHub only yet.
Exit codes: 0 ok, 1 none or refused.
Example:
  ferrule issue search --state open modbus timeout
See: ferrule guide show issue";

const ISSUE_VIEW_AFTER_LONG_HELP: &str = "\
Output (stdout; compact text, styled on a terminal):
  #<n> <title> [<STATE>]         Line 1: the head (`KEY-1 <summary> [<status>]`
                                 for Jira), then the URL when the host has
                                 one and `by <author>`
  <body>                         The description
  --- <n> comment(s) ---         Then each comment as `[<author> @ <time>]`
                                 and its text
Filters (the head line, URL and author always print):
  --body                         Only the body
  --comments                     Only the comments; with --body, both
  --last <n>                     Only the latest n comments
  --author <login>               Only the comments by that author, ignoring
                                 case
  A filtered count reads `--- <k> of <n> comment(s) ---`.
Examples:
  ferrule issue view <n> --comments --last 3
  ferrule issue view <n> --body
Refusals (stdout, exit 1):
  issue=refused reason=pull-request hint=ferrule pr view <n>
                                 The number names a pull request (GitHub
                                 serves PRs as issues); view it with pr view.
Errors (stderr, exit 1): `issue view: <the host's error>` for an unknown issue, a
missing credential or a failing `gh`/`curl`.
Exit codes: 0 ok, 1 refused.
Backend: reads from the configured tracker (`backend` in ferrule.toml).";

const ISSUE_REQUEST_AFTER_LONG_HELP: &str = "\
Output (stdout): the requested part as plain text, nothing when the issue has
no such part. With `--comments`, the comments in the form `issue view` prints
them (`[<author> @ <time>]` then the text).
Errors (stderr, exit 1): `issue request: <the tracker's error>` for an unknown
issue, a missing credential or a failing `gh`/`curl`.
Exit codes: 0 ok, 1 refused.
Backend: reads from the configured tracker.
See: ferrule guide show epic";

const ISSUE_BODY_AFTER_LONG_HELP: &str = "\
Output (stdout): the body, line 1 `# <title>`, nothing else. Reads the run's
files and cards; posts nothing.
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
See: ferrule guide show issue";

const ISSUE_POST_AFTER_LONG_HELP: &str = "\
Output:
  issue=<n> action=<action>
      Filed or updated issue <n>; the card's `issue:` (or `slices:` /
      `split:`) records it. <action> is one of:
        created    A new issue was filed
        adopted    An open issue with the same title became the run's
        edited     The tracking issue's title and body were replaced (an
                   epic's, or `--reuse`)
        commented  `--comment` posted issue-comment.md
        recorded   The card already names the issue; nothing was posted
Refusals (stdout, exit 1):
  post=refused reason=card hint=no card for this slug; check it with `ferrule board list`
      No card for the slug. Fix: check the slug with `ferrule board list`.
  post=refused reason=no-issue hint=file the tracking issue first: ferrule issue post <slug>
      `--split`, `--follow-up` or `--comment` before the card has an issue.
      Fix: `ferrule issue post <slug>` first.
  body=refused stage-ref=<token> hint=reword the source text so it names no stage or agent
      The tracking or slice body names a stage or an agent. Exit 3. Fix:
      reword the source file (see `issue body`).
Errors (stderr, exit 1): `missing <path>` for an absent run file; the tracker's
error when the host call fails.
Exit codes: 0 ok, 1 refused, 3 content.
Backend: writes to the configured tracker (`gh`, Jira or Bitbucket).
See: ferrule guide show issue";

/// What `issue view` and `pr view` print; without a flag, everything.
#[derive(Debug, Clone, Args)]
pub struct ViewArgs {
    /// Print the body; with `--comments` too, both. The head line, URL and
    /// author always print.
    #[arg(long)]
    pub body: bool,
    /// Print the comments.
    #[arg(long)]
    pub comments: bool,
    /// Keep only the latest n comments.
    #[arg(long, value_name = "N")]
    pub last: Option<usize>,
    /// Keep only the comments by this author, ignoring case.
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
    /// Print an issue's title, body and comments as compact text.
    #[command(after_long_help = ISSUE_VIEW_AFTER_LONG_HELP)]
    View {
        /// A GitHub number or URL, or a Jira key.
        issue: String,
        #[command(flatten)]
        view: ViewArgs,
    },
    /// Print part of a reused issue, for the run's request.md.
    ///
    /// Without a flag: the issue's original request.
    #[command(after_long_help = ISSUE_REQUEST_AFTER_LONG_HELP)]
    Request {
        /// A GitHub number or URL, or a Jira key.
        issue: String,
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
        slug: String,
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
        slug: String,
        #[command(flatten)]
        target: FileTarget,
        /// Reuse issue n as the tracking issue: its title and body are
        /// replaced.
        #[arg(long, value_name = "N", conflicts_with_all = ["slice", "split", "follow_up", "comment"])]
        reuse: Option<u64>,
    },
}

#[derive(Debug, Clone, Copy, clap::Args)]
#[group(multiple = false)]
pub struct FileTarget {
    /// File slice n of slices.md; appended to the card's `slices:`.
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
        } => file(slug, *target, *reuse, out),
    }
}

fn search(keywords: &[String], state: SearchState, out: &mut impl Write) -> io::Result<u8> {
    let config = config()?;
    let max = config.thresholds.search_max;
    let state = match state {
        SearchState::Open => "open",
        SearchState::Closed => "closed",
        SearchState::All => "all",
    };
    match remote::issue_search(
        config.backend.tracker,
        keywords,
        state,
        max + 1,
        &project::root(),
    ) {
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
    Config::load(&project::main_checkout()).map_err(sql)
}

/// Prints an issue or PR: compact text, or styled with its body and
/// comments rendered.
pub fn print_item(
    item: &remote::Item,
    select: &remote::Select,
    style: Option<Style>,
    out: &mut impl Write,
) -> io::Result<()> {
    let Some(s) = style else {
        return write!(out, "{}", item.text_for(select));
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
    if !select.shows_comments() {
        return Ok(());
    }
    writeln!(
        out,
        "\n{}",
        term::faint(&format!("── {} ──", select.count(item)), s)
    )?;
    for c in select.comments(item) {
        writeln!(
            out,
            "\n{}",
            term::strong(&format!("{} · {}", c.author, c.at), s)
        )?;
        write!(out, "{}", term::markdown(&c.body, s))?;
    }
    Ok(())
}

fn view(
    issue: &str,
    view: &ViewArgs,
    style: Option<Style>,
    out: &mut impl Write,
) -> io::Result<u8> {
    match remote::issue(config()?.backend.tracker, issue, &project::root()) {
        Ok(item) => {
            print_item(&item, &view.select(), style, out)?;
            Ok(exit::OK)
        }
        Err(remote::Error::PullRequest(n)) => {
            writeln!(
                out,
                "issue=refused reason=pull-request hint=ferrule pr view {n}"
            )?;
            Ok(exit::REFUSED)
        }
        Err(e) => {
            eprintln!("issue view: {e}");
            Ok(exit::REFUSED)
        }
    }
}

fn request(
    issue: &str,
    part: RequestPart,
    style: Option<Style>,
    out: &mut impl Write,
) -> io::Result<u8> {
    let item = match remote::issue(config()?.backend.tracker, issue, &project::root()) {
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
        Err(body::Error::StageRef(t)) => {
            writeln!(
                out,
                "body=refused stage-ref={t} \
                 hint=reword the source text so it names no stage or agent"
            )?;
            Ok(exit::CONTENT)
        }
        Err(body::Error::Missing(p)) => {
            eprintln!("body: missing {}", p.display());
            Ok(exit::REFUSED)
        }
        Err(body::Error::NoSlice(n)) => {
            eprintln!("body: no slice {n} in slices.md");
            Ok(exit::REFUSED)
        }
        Err(body::Error::Empty) => Ok(exit::REFUSED),
        Err(body::Error::Sql(e)) => Err(sql(e)),
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
        None => ferrule_core::store::Store::memory().map_err(sql)?,
    };
    print_body(body::issue(&s.conn, &layout, slug, slice), style, out)
}

/// Prints the refusal of a post; its exit code.
pub fn post_failed(e: publish::Error, out: &mut impl Write) -> io::Result<u8> {
    match e {
        publish::Error::Body(b) => print_body(Err(b), None, out),
        publish::Error::Missing(p) => {
            eprintln!("missing {}", p.display());
            Ok(exit::REFUSED)
        }
        publish::Error::NoIssue => {
            writeln!(
                out,
                "post=refused reason=no-issue \
                 hint=file the tracking issue first: ferrule issue post <slug>"
            )?;
            Ok(exit::REFUSED)
        }
        publish::Error::NoBranch => {
            writeln!(
                out,
                "post=refused reason=no-branch \
                 hint=approve gate2 first: ferrule gate approve <slug> gate2"
            )?;
            Ok(exit::REFUSED)
        }
        publish::Error::Card => {
            writeln!(
                out,
                "post=refused reason=card hint=no card for this slug; check it with `ferrule board list`"
            )?;
            Ok(exit::REFUSED)
        }
        publish::Error::Remote(e) => {
            eprintln!("{e}");
            Ok(exit::REFUSED)
        }
        publish::Error::Sql(e) => Err(sql(e)),
    }
}

fn file(slug: &str, t: FileTarget, reuse: Option<u64>, out: &mut impl Write) -> io::Result<u8> {
    let target = match (t.slice, t.split, t.follow_up, t.comment) {
        (Some(n), ..) => Target::Slice(n),
        (_, true, ..) => Target::Split,
        (_, _, Some(k), _) => Target::FollowUp(k),
        (.., true) => Target::Comment,
        _ => Target::Run,
    };
    let layout = Layout::new(project::main_checkout());
    let config = config()?;
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
        config: &config,
        root: &layout.root,
        slug,
        now: &now,
    };
    match run.file(target, reuse) {
        Ok((n, action)) => {
            writeln!(out, "issue={n} action={}", action.as_str())?;
            Ok(exit::OK)
        }
        Err(e) => post_failed(e, out),
    }
}
