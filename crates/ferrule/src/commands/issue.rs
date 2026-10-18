use std::io::{self, Write};

use clap::Subcommand;
use ferrule_core::body;
use ferrule_core::card::board;
use ferrule_core::config::Config;
use ferrule_core::issue::request::{self, Part};
use ferrule_core::pr::publish::{self, Target};
use ferrule_core::project::{self, Layout};
use ferrule_core::remote;
use ferrule_core::term::{self, Style};

use super::card::{sql, store};
use super::exit;

#[derive(Debug, Subcommand)]
pub enum IssueCommand {
    /// Print an issue's title, body and comments as compact text.
    View {
        /// A GitHub number or URL, or a Jira key.
        issue: String,
    },
    /// Print part of a reused issue, for the run's request.md.
    ///
    /// Without a flag: the issue's original request.
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
    Body {
        /// The run's slug.
        slug: String,
        /// Slice n's own issue instead.
        #[arg(long, value_name = "N")]
        slice: Option<u32>,
    },
    /// File the run's issue, or a related one, and record its number.
    ///
    /// An open issue with the same title is adopted, never filed twice.
    File {
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
        IssueCommand::View { issue } => view(issue, style, out),
        IssueCommand::Request { issue, part } => request(issue, *part, style, out),
        IssueCommand::Body { slug, slice } => body(slug, *slice, style, out),
        IssueCommand::File {
            slug,
            target,
            reuse,
        } => file(slug, *target, *reuse, out),
    }
}

fn config() -> io::Result<Config> {
    Config::load(&project::main_checkout()).map_err(sql)
}

/// Prints an issue or PR: compact text, or styled with its body and
/// comments rendered.
pub fn print_item(
    item: &remote::Item,
    style: Option<Style>,
    out: &mut impl Write,
) -> io::Result<()> {
    let Some(s) = style else {
        return write!(out, "{}", item.text());
    };
    writeln!(out, "{}", term::strong(&item.head, s))?;
    let by = match &item.url {
        Some(url) => format!("{url} · by {}", item.by),
        None => format!("by {}", item.by),
    };
    writeln!(out, "{}\n", term::faint(&by, s))?;
    write!(out, "{}", term::markdown(&item.body, s))?;
    let n = item.comments.len();
    writeln!(
        out,
        "\n{}",
        term::faint(&format!("── {n} comment(s) ──"), s)
    )?;
    for c in &item.comments {
        writeln!(
            out,
            "\n{}",
            term::strong(&format!("{} · {}", c.author, c.at), s)
        )?;
        write!(out, "{}", term::markdown(&c.body, s))?;
    }
    Ok(())
}

fn view(issue: &str, style: Option<Style>, out: &mut impl Write) -> io::Result<u8> {
    match remote::issue(config()?.backend.tracker, issue, &project::root()) {
        Ok(item) => {
            print_item(&item, style, out)?;
            Ok(exit::OK)
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
            writeln!(out, "body=refused stage-ref={t}")?;
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
            writeln!(out, "post=refused reason=no-issue")?;
            Ok(exit::REFUSED)
        }
        publish::Error::NoBranch => {
            writeln!(out, "post=refused reason=no-branch")?;
            Ok(exit::REFUSED)
        }
        publish::Error::Card => {
            writeln!(out, "post=refused reason=card")?;
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
        writeln!(out, "post=refused reason=card")?;
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
