//! The tracking issue's and the pull request's bodies, assembled from a
//! run's artifacts and cards. The issue holds what the user gets; the PR
//! holds how it is built. Line 1 of either is `# <title>`.
//!
//! Neither body may name a stage (`s3`) or an agent in its visible text:
//! that is workflow vocabulary a reader of the issue never sees.

use std::collections::HashSet;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use rusqlite::Connection;

use crate::card::board::{self, State};
use crate::project::Layout;
use crate::spec::{
    self,
    diff::{Part, Render, SpecDiff},
};
use crate::{artifacts, git};

#[derive(Debug)]
pub enum Error {
    /// An artifact the body needs is absent.
    Missing(PathBuf),
    /// slices.md has no such slice.
    NoSlice(u32),
    /// The visible text names a stage or an agent: the token found.
    StageRef(String),
    /// Nothing to print.
    Empty,
    Sql(rusqlite::Error),
}

impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Self {
        Error::Sql(e)
    }
}

const AGENTS: [&str; 6] = [
    "fe-author",
    "fe-architect",
    "fe-planner",
    "fe-implementer",
    "fe-reviewer",
    "orchestrator",
];

/// The first stage id or agent name in the visible text. HTML comments,
/// fenced blocks, code spans holding `.` or `/` and the original request
/// are not scanned.
#[must_use]
pub fn stage_ref(text: &str) -> Option<String> {
    let mut fence = false;
    let mut request = false;
    for line in text.lines() {
        if line.starts_with("```") {
            fence = !fence;
            continue;
        }
        if fence {
            continue;
        }
        if line.starts_with("<details><summary>Original request</summary>") {
            request = true;
        }
        if request {
            if line.starts_with("</details>") {
                request = false;
            }
            continue;
        }
        let visible = visible(line);
        if let Some(t) = stage_word(&visible) {
            return Some(t);
        }
        let lower = visible.to_ascii_lowercase();
        if let Some((_, agent)) = AGENTS
            .iter()
            .filter_map(|a| lower.find(a).map(|j| (j, *a)))
            .min()
        {
            return Some(agent.to_owned());
        }
    }
    None
}

/// The line without HTML comments and without code spans that hold `.`
/// or `/` (paths, file names), each span replaced by a space.
fn visible(line: &str) -> String {
    let mut s = String::new();
    let mut rest = line;
    while let Some(i) = rest.find("<!--") {
        s.push_str(&rest[..i]);
        rest = rest[i..].find("-->").map_or("", |j| &rest[i + j + 3..]);
    }
    s.push_str(rest);
    let mut out = String::new();
    for (i, seg) in s.split('`').enumerate() {
        if i % 2 == 0 || !seg.contains(['.', '/']) {
            out.push_str(seg);
            out.push(' ');
        }
    }
    out
}

/// `s<digits>` as a whole word.
fn stage_word(line: &str) -> Option<String> {
    let b = line.as_bytes();
    let word = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    for i in 0..b.len() {
        if b[i] != b's' || (i > 0 && word(b[i - 1])) {
            continue;
        }
        let d = b[i + 1..].iter().take_while(|c| c.is_ascii_digit()).count();
        if d > 0 && b.get(i + 1 + d).is_none_or(|&c| !word(c)) {
            return Some(line[i..=i + d].to_owned());
        }
    }
    None
}

/// Text without its trailing blank lines, ending in one newline.
fn trim_end(text: &str) -> String {
    let t = text.trim_end_matches('\n');
    if t.is_empty() {
        String::new()
    } else {
        format!("{t}\n")
    }
}

fn finish(text: &str) -> Result<String, Error> {
    let out = trim_end(text);
    if out.is_empty() {
        return Err(Error::Empty);
    }
    if let Some(t) = stage_ref(&out) {
        return Err(Error::StageRef(t));
    }
    Ok(out)
}

/// The run's artifacts and parent card.
struct Ctx<'a> {
    conn: &'a Connection,
    dir: PathBuf,
    slug: &'a str,
}

impl Ctx<'_> {
    fn read(&self, name: &str) -> Option<String> {
        std::fs::read_to_string(self.dir.join(name)).ok()
    }

    fn need(&self, name: &str) -> Result<String, Error> {
        self.read(name)
            .ok_or_else(|| Error::Missing(self.dir.join(name)))
    }

    fn field(&self, key: &str) -> Result<Option<String>, Error> {
        Ok(board::field(self.conn, self.slug, key)?.filter(|v| !v.is_empty()))
    }

    fn spec(&self) -> Option<SpecDiff> {
        self.read("spec-diff.md").map(|t| SpecDiff::parse(&t))
    }
}

fn first_line(text: &str) -> &str {
    text.lines().next().unwrap_or("")
}

/// `[71, 72]` → its n-th item, 1-based.
fn nth(list: Option<&str>, n: usize) -> Option<String> {
    list?
        .trim_matches(|c| c == '[' || c == ']')
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .nth(n.checked_sub(1)?)
        .map(str::to_owned)
}

/// An issue.md section's lines without blanks, under its heading.
fn section(issue: &str, name: &str) -> String {
    let heading = format!("## {name}");
    let body: Vec<&str> = issue
        .lines()
        .skip_while(|l| *l != heading)
        .skip(1)
        .take_while(|l| !l.starts_with("## "))
        .filter(|l| !l.is_empty())
        .collect();
    if body.is_empty() {
        String::new()
    } else {
        format!("## {name}\n{}\n\n", body.join("\n"))
    }
}

/// One slice of slices.md: its title and `key: value` lines.
struct Slice {
    n: u32,
    title: String,
    fields: Vec<(String, String)>,
}

impl Slice {
    fn get(&self, key: &str) -> &str {
        self.fields
            .iter()
            .find(|(k, _)| k == key)
            .map_or("", |(_, v)| v.as_str())
    }
}

fn slices(text: &str) -> Vec<Slice> {
    let mut out: Vec<Slice> = Vec::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("## Slice ")
            && let Some((n, title)) = rest.split_once(": ")
            && let Ok(n) = n.parse()
        {
            out.push(Slice {
                n,
                title: title.to_owned(),
                fields: Vec::new(),
            });
        } else if let Some(s) = out.last_mut()
            && let Some((k, v)) = line.split_once(": ")
            && !k.is_empty()
            && !k.starts_with(['#', '-', ' '])
        {
            s.fields.push((k.to_owned(), v.to_owned()));
        }
    }
    out
}

fn render(diff: &SpecDiff, part: Part, only: Option<Vec<String>>) -> String {
    spec::diff::render(
        diff,
        part,
        &Render {
            only,
            first: false,
            issue_ids: None,
        },
    )
}

/// `Decide:` line, the summary's component table, the public surface.
fn surface(summary: &str) -> Vec<&str> {
    let mut on = false;
    let mut out = Vec::new();
    for l in summary.lines() {
        if l.starts_with("Public surface:") {
            on = true;
        } else if on && l.starts_with("- ") {
            if l != "- none" {
                out.push(l);
            }
        } else {
            on = false;
        }
    }
    out
}

fn decide(summary: &str) -> &str {
    summary
        .lines()
        .find_map(|l| l.strip_prefix("Decide: "))
        .unwrap_or("")
}

/// A `## <name>` section of a markdown file outside fenced blocks, fences
/// inside it kept.
fn md_section<'a>(text: &'a str, name: &str) -> Vec<&'a str> {
    let heading = format!("## {name}");
    let mut fence = false;
    let mut on = false;
    let mut out = Vec::new();
    for l in text.lines() {
        if l.starts_with("```") {
            fence = !fence;
        }
        if !fence && l.starts_with("## ") {
            on = l == heading;
            continue;
        }
        if on {
            out.push(l);
        }
    }
    out
}

/// The tracking issue's body, or slice `slice`'s own issue.
pub fn issue(
    conn: &Connection,
    layout: &Layout,
    slug: &str,
    slice: Option<u32>,
) -> Result<String, Error> {
    let c = Ctx {
        conn,
        dir: layout.run_dir(slug),
        slug,
    };
    let issue = c.need("issue.md")?;
    let text = match slice {
        Some(n) => slice_issue(&c, &issue, n)?,
        None => run_issue(&c, &issue)?,
    };
    finish(&text)
}

fn slice_issue(c: &Ctx, issue: &str, n: u32) -> Result<String, Error> {
    let all = slices(&c.need("slices.md")?);
    let Some(s) = all.iter().find(|s| s.n == n) else {
        return Err(Error::NoSlice(n));
    };
    let mut out = format!("{}: {}\n\n", first_line(issue), s.title);
    let slices_field = c.field("slices")?;
    let deps: Vec<String> = s
        .get("Depends on")
        .split(',')
        .map(str::trim)
        .filter(|d| !d.is_empty() && *d != "none")
        .map(|d| {
            let num = d.parse().ok().and_then(|i| nth(slices_field.as_deref(), i));
            num.map_or_else(|| format!("slice {d}"), |n| format!("#{n}"))
        })
        .collect();
    let deps = if deps.is_empty() {
        "none".to_owned()
    } else {
        deps.join(", ")
    };
    let _ = write!(
        out,
        "**Part of epic** #{} · **Slice** {n} of {} · **Depends on** {deps}\n\n",
        c.field("issue")?.as_deref().unwrap_or("?"),
        all.len()
    );
    let _ = write!(out, "## Goal\n{}\n\n", s.get("Goal"));
    let only: Vec<String> = format!("{} {}", s.get("Scenarios"), s.get("Entries"))
        .split([',', ' '])
        .filter(|w| !w.is_empty())
        .map(str::to_owned)
        .collect();
    if let Some(diff) = c.spec() {
        let only = Some(only).filter(|o| !o.is_empty());
        out.push_str(&render(&diff, Part::Requirements, only.clone()));
        out.push_str(&render(&diff, Part::Scenarios, only));
    }
    Ok(out)
}

fn run_issue(c: &Ctx, issue: &str) -> Result<String, Error> {
    let kind = c.field("kind")?;
    let mut out = format!("{}\n\n", first_line(issue));
    if kind.as_deref() == Some("slice")
        && let Some(line) = c.read("slice-line.md").filter(|t| !t.is_empty())
    {
        let _ = write!(out, "{}\n\n", first_line(&line));
    }
    out.push_str(&section(issue, "Goal"));
    if let Some(diff) = c.spec() {
        out.push_str(&render(&diff, Part::Requirements, None));
        out.push_str(&render(&diff, Part::Scenarios, None));
    }
    out.push_str(&section(issue, "Out of scope"));
    if kind.as_deref() == Some("epic") {
        epic_parts(c, &mut out)?;
    }
    if let Some(req) = c.read("request.md").filter(|t| !t.is_empty()) {
        let _ = write!(
            out,
            "<details><summary>Original request</summary>\n\n{}{}</details>\n",
            req,
            if req.ends_with('\n') { "" } else { "\n" }
        );
    }
    Ok(out)
}

fn epic_parts(c: &Ctx, out: &mut String) -> Result<(), Error> {
    if let Some(summary) = c.read("architecture.summary.md") {
        let _ = write!(out, "## Architecture\n{}\n\n", decide(&summary));
        for l in summary.lines().filter(|l| l.starts_with('|')) {
            let _ = writeln!(out, "{l}");
        }
        out.push('\n');
        let surface = surface(&summary);
        if !surface.is_empty() {
            let _ = write!(out, "Public surface:\n{}\n\n", surface.join("\n"));
        }
        if let Some(arch) = c.read("architecture.md") {
            out.push_str("### Interfaces\n");
            for l in md_section(&arch, "Interfaces") {
                match l.strip_prefix("### ") {
                    Some(r) => {
                        let _ = writeln!(out, "#### {r}");
                    }
                    None => {
                        let _ = writeln!(out, "{l}");
                    }
                }
            }
        }
    }
    if let Some(text) = c.read("slices.md") {
        let slices_field = c.field("slices")?;
        out.push_str("## Slices\n");
        for s in slices(&text) {
            let link = nth(slices_field.as_deref(), s.n as usize)
                .map_or_else(|| "not filed yet".to_owned(), |n| format!("#{n}"));
            let _ = writeln!(out, "- [ ] **Slice {}: {}** — {link}", s.n, s.title);
            let _ = writeln!(out, "  - {}", s.get("Goal"));
            let _ = writeln!(out, "  - depends on: {}", s.get("Depends on"));
        }
        out.push('\n');
    }
    Ok(())
}

/// The PR body's form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Form {
    /// From the first push to gate 4: progress, rendered again on every
    /// board move.
    Draft,
    /// At gate 4, from pr.md.
    Full,
}

/// A card's `spec-delta <ID> <kind> [origin=…] [quote=…] <text>` line.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Delta {
    id: String,
    kind: String,
    origin: String,
    quote: String,
    text: String,
}

fn delta(body: &str) -> Option<Delta> {
    let rest = body.strip_prefix("spec-delta ")?;
    let (id, rest) = rest.split_once(' ')?;
    let (kind, mut rest) = rest.split_once(' ').unwrap_or((rest, ""));
    let (mut origin, mut quote) = (String::new(), String::new());
    loop {
        rest = rest.trim_start_matches(' ');
        let (key, value) = if let Some(v) = rest.strip_prefix("origin=") {
            (&mut origin, v)
        } else if let Some(v) = rest.strip_prefix("quote=") {
            (&mut quote, v)
        } else {
            break;
        };
        let (val, after) = match value.strip_prefix('"') {
            Some(q) => q.split_once('"').unwrap_or((q, "")),
            None => value.split_once(' ').unwrap_or((value, "")),
        };
        val.clone_into(key);
        rest = after;
    }
    Some(Delta {
        id: id.to_owned(),
        kind: kind.to_owned(),
        origin,
        quote,
        text: rest.to_owned(),
    })
}

/// `## Spec changes since filing` from change lines `- <ID> …`: each ID
/// gets its local ID, and a user's change its quoted words. Returns the IDs
/// listed.
fn spec_changes(
    diff: &SpecDiff,
    deltas: &[Delta],
    lines: &[String],
    out: &mut String,
) -> HashSet<String> {
    let mut ids = HashSet::new();
    if lines.is_empty() {
        return ids;
    }
    out.push_str("## Spec changes since filing\n");
    for line in lines {
        let mut line = line.clone();
        if let Some(rest) = line.strip_prefix("- ") {
            let id: String = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
                .collect();
            if crate::spec::fixed_id_len(id.as_bytes()) == Some(id.len()) {
                ids.insert(id.clone());
                let mut l = format!("- {id}");
                if let Some(local) = diff.was.get(&id) {
                    let _ = write!(l, " ({local})");
                }
                l.push_str(&rest[id.len()..]);
                let quote = deltas
                    .iter()
                    .rev()
                    .find(|d| d.id == id && !d.quote.is_empty());
                if let Some(q) = quote
                    && let Some(head) = l.strip_suffix(" — user")
                {
                    l = format!("{head} — user: \"{}\"", q.quote);
                }
                line = l;
            }
        }
        let _ = writeln!(out, "{line}");
    }
    out.push('\n');
    ids
}

/// decisions.md's design choices and its chosen spec decisions not already
/// under the spec changes.
fn decisions(text: Option<&str>, changed: &HashSet<String>) -> String {
    let mut seen: Vec<String> = Vec::new();
    for line in text.unwrap_or("").lines() {
        let Some(item) = line.strip_prefix("- ") else {
            continue;
        };
        let found = ["(chosen)", "(user)"].iter().find_map(|w| {
            let at = item.find(&format!(" {w} ["))?;
            let after = &item[at + w.len() + 3..];
            let tag = after.split_once(']')?.0;
            let ok = matches!(tag, "design" | "process" | "scope")
                || tag.strip_prefix("spec ").is_some_and(|id| {
                    !id.is_empty()
                        && id
                            .bytes()
                            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'-')
                });
            ok.then(|| (&item[..at], &w[1..w.len() - 1], tag))
        });
        let Some((choice, who, tag)) = found else {
            continue;
        };
        let keep = tag == "design"
            || tag
                .strip_prefix("spec ")
                .is_some_and(|id| who == "chosen" && !changed.contains(id));
        let r = format!("- {choice} ({who})");
        if keep && !seen.contains(&r) {
            seen.push(r);
        }
    }
    if seen.is_empty() {
        String::new()
    } else {
        format!("## Decisions\n{}\n\n", seen.join("\n"))
    }
}

/// The architecture.md component table.
fn architecture(c: &Ctx) -> String {
    let Some(summary) = c.read("architecture.summary.md") else {
        return String::new();
    };
    let mut out = format!("## Architecture\n{}\n\n", decide(&summary));
    if let Some(arch) = c.read("architecture.md") {
        let table: Vec<&str> = md_section(&arch, "Components")
            .into_iter()
            .filter(|l| l.starts_with('|'))
            .collect();
        if !table.is_empty() {
            let _ = write!(out, "{}\n\n", table.join("\n"));
        }
    }
    let surface = surface(&summary);
    if !surface.is_empty() {
        let _ = writeln!(out, "Public surface:\n{}", surface.join("\n"));
    }
    out.push('\n');
    out
}

fn scenarios(diff: Option<&SpecDiff>) -> Option<(usize, String)> {
    let diff = diff?;
    (!diff.scenarios.is_empty()).then(|| {
        (
            diff.scenarios.len(),
            diff.scenarios
                .iter()
                .map(|s| format!("{} {}", s.id, s.title))
                .collect::<Vec<_>>()
                .join(" · "),
        )
    })
}

/// The pull request's body.
pub fn pr(
    conn: &Connection,
    layout: &Layout,
    slug: &str,
    form: Form,
    main_branch: &str,
) -> Result<String, Error> {
    let c = Ctx {
        conn,
        dir: layout.run_dir(slug),
        slug,
    };
    let issue = c.need("issue.md")?;
    let deltas: Vec<Delta> = board::run_events(conn, slug)?
        .iter()
        .filter_map(|(_, _, b)| delta(b))
        .collect();
    let mut out = match form {
        Form::Draft => draft(&c, &issue, &deltas, &layout.root, main_branch)?,
        Form::Full => full(&c, &issue, &deltas)?,
    };
    if let Some(i) = c.field("issue")? {
        let _ = writeln!(out, "Closes #{i}");
    }
    finish(&out)
}

fn spec_rev(c: &Ctx) -> Option<u32> {
    c.read("spec-diff.md").and_then(|t| artifacts::rev(&t))
}

fn issue_ref(c: &Ctx) -> Result<String, Error> {
    Ok(c.field("issue")?
        .map_or_else(|| "the tracking issue".to_owned(), |i| format!("#{i}")))
}

/// The requirements table with its Issue column.
fn requirements(c: &Ctx, diff: Option<&SpecDiff>) -> Result<String, Error> {
    let Some(diff) = diff else {
        return Ok(String::new());
    };
    let req = spec::diff::render(
        diff,
        Part::Entries,
        &Render {
            only: None,
            first: false,
            issue_ids: Some(board::reworded_ids(c.conn, c.slug)?),
        },
    );
    let req = trim_end(&req);
    Ok(if req.is_empty() {
        req
    } else {
        format!("{req}\n")
    })
}

fn draft(
    c: &Ctx,
    issue: &str,
    deltas: &[Delta],
    root: &Path,
    main_branch: &str,
) -> Result<String, Error> {
    let diff = c.spec();
    let mut out = format!("{}\n\n", first_line(issue));
    let mut status = "Draft".to_owned();
    if let Some(lane) = c.field("lane")? {
        let _ = write!(status, " · lane {lane}");
    }
    match spec_rev(c) {
        Some(r) => {
            let _ = write!(status, " · spec rev {r}");
        }
        None => status.push_str(" · no spec change"),
    }
    if let Some(sha) = board::pass_shas(c.conn, c.slug)?.pop() {
        let _ = write!(status, " · gauntlet green at {sha}");
    }
    let goal = issue
        .lines()
        .skip_while(|l| *l != "## Goal")
        .nth(1)
        .unwrap_or("");
    let _ = write!(out, "{status}\n\n## Why\n{goal}\n\n");
    if let Some((n, list)) = scenarios(diff.as_ref()) {
        let _ = write!(
            out,
            "## Scenarios\nThe {n} scenarios of {}: {list}\n\n",
            issue_ref(c)?
        );
    }
    let summary = c.read("plan.summary.md").unwrap_or_default();
    stages(c, &summary, &mut out)?;
    review_here(c, &summary, &mut out)?;
    out.push_str(&requirements(c, diff.as_ref())?);
    let lines = live_changes(c, deltas, root, main_branch)?;
    let changed = match &diff {
        Some(d) => spec_changes(d, deltas, &lines, &mut out),
        None => HashSet::new(),
    };
    out.push_str(&architecture(c));
    out.push_str(&decisions(c.read("decisions.md").as_deref(), &changed));
    Ok(out)
}

/// `(sid, what)` of the plan summary's stage table.
fn plan_stages(summary: &str) -> Vec<(String, String)> {
    summary
        .lines()
        .filter(|l| {
            l.strip_prefix("| s")
                .is_some_and(|r| r.starts_with(|c: char| c.is_ascii_digit()))
        })
        .map(|l| {
            let cells: Vec<&str> = l.split('|').collect();
            (
                cells.get(1).map_or("", |s| s.trim()).replace(' ', ""),
                cells.get(2).map_or("", |s| s.trim()).to_owned(),
            )
        })
        .collect()
}

/// A stage's verification beyond unit tests, as sub-items of its line.
fn verification(summary: &str, sid: &str) -> Vec<String> {
    let mut on = false;
    let mut cur = false;
    let mut out = Vec::new();
    for l in summary.lines() {
        if l.starts_with("Verification beyond unit tests:") {
            on = true;
        } else if on && let Some(item) = l.strip_prefix("- ") {
            let (ids, rest) = item.split_once(" — ").unwrap_or((item, ""));
            let listed = ids.split(", ").all(|id| {
                id.strip_prefix('s')
                    .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
            });
            cur = listed && ids.split(", ").any(|id| id == sid);
            if cur {
                out.push(format!("  - beyond unit tests: {rest}"));
            }
        } else if on && l.starts_with("  - ") {
            if cur {
                out.push(format!("  {l}"));
            }
        } else {
            on = false;
        }
    }
    out
}

fn stages(c: &Ctx, summary: &str, out: &mut String) -> Result<(), Error> {
    out.push_str("## Stages\n");
    let cards = board::stages(c.conn, c.slug)?;
    for (sid, what) in plan_stages(summary) {
        let card = cards.iter().find(|(n, _, _)| format!("s{n}") == sid);
        let (state, commit, blocked) = match card {
            Some((_, id, st)) => (
                Some(*st),
                board::last_token(c.conn, id, "commit")?,
                board::field(c.conn, id, "blocked-by")?.unwrap_or_default(),
            ),
            None => (None, None, String::new()),
        };
        let blockers: Vec<&str> = blocked
            .trim_matches(|ch| ch == '[' || ch == ']')
            .split(',')
            .map(str::trim)
            .filter(|b| !b.is_empty())
            .collect();
        let short: Vec<&str> = blockers
            .iter()
            .map(|b| b.rsplit_once('.').map_or(*b, |(_, s)| s))
            .collect();
        let mark = if short.is_empty() {
            format!("<!-- {sid} -->")
        } else {
            format!("<!-- {sid} blocked-by {} -->", short.join(","))
        };
        let line = match state {
            Some(State::Done) => format!(
                "- [x] {what} ({}) {mark}",
                commit.as_deref().unwrap_or("no commit")
            ),
            Some(State::InReview) => format!("- [ ] {what} — in review {mark}"),
            Some(State::InProgress) => format!("- [ ] {what} — implementing {mark}"),
            Some(State::Open) => {
                let mut open = false;
                for b in &blockers {
                    open |= board::state(c.conn, b)? != Some(State::Done);
                }
                let word = if open { "blocked" } else { "next" };
                format!("- [ ] {what} — {word} {mark}")
            }
            None => format!("- [ ] {what} {mark}"),
        };
        let _ = writeln!(out, "{line}");
        for v in verification(summary, &sid) {
            let _ = writeln!(out, "{v}");
        }
    }
    out.push('\n');
    Ok(())
}

fn review_here(c: &Ctx, summary: &str, out: &mut String) -> Result<(), Error> {
    out.push_str("## Review here\n");
    let plan = plan_stages(summary);
    let landed: Vec<&str> = board::stages(c.conn, c.slug)?
        .iter()
        .filter(|(_, _, st)| *st == State::Done)
        .filter_map(|(n, _, _)| {
            plan.iter()
                .find(|(sid, _)| *sid == format!("s{n}"))
                .map(|(_, w)| w.as_str())
        })
        .collect();
    if landed.is_empty() {
        out.push_str(
            "- nothing on the branch yet — inline comments are answered once a stage lands\n",
        );
    } else {
        let _ = writeln!(
            out,
            "- on the branch: {} — inline comments on their files are answered before the next stop",
            landed.join(", ")
        );
    }
    if c.field("lane")?.as_deref() == Some("fast") {
        out.push_str("- later stages land on green, no stop until gate 4\n");
    } else {
        out.push_str("- later stages land after their review and the user's approval\n");
    }
    out.push('\n');
    Ok(())
}

/// The cards' spec-delta lines for the IDs the committed branch diff
/// touches, the last line per ID, as `- <ID> <kind>: <text> — <origin>`.
fn live_changes(
    c: &Ctx,
    deltas: &[Delta],
    root: &Path,
    main_branch: &str,
) -> Result<Vec<String>, Error> {
    let Some(branch) = c.field("branch")? else {
        return Ok(Vec::new());
    };
    let range = format!("{main_branch}...{branch}");
    let Some(diff) = git::out(root, &["diff", &range, "--", "docs/specs"]) else {
        return Ok(Vec::new());
    };
    let mut touched: HashSet<String> = HashSet::new();
    for l in diff.lines().filter(|l| {
        (l.starts_with('+') || l.starts_with('-')) && !l.starts_with("+++") && !l.starts_with("---")
    }) {
        for part in l.split("**").skip(1).step_by(2) {
            if crate::spec::fixed_id_len(part.as_bytes()) == Some(part.len()) {
                touched.insert(part.to_owned());
            }
        }
    }
    let mut order: Vec<(&str, String)> = Vec::new();
    for d in deltas.iter().filter(|d| touched.contains(&d.id)) {
        order.retain(|(id, _)| *id != d.id);
        order.push((
            &d.id,
            format!("- {} {}: {} — {}", d.id, d.kind, d.text, d.origin),
        ));
    }
    Ok(order.into_iter().map(|(_, l)| l).collect())
}

fn full(c: &Ctx, issue: &str, deltas: &[Delta]) -> Result<String, Error> {
    let pr = c.need("pr.md")?;
    let diff = c.spec();
    let mut status = match spec_rev(c) {
        Some(r) => format!("Spec rev {r}"),
        None => "No spec change".to_owned(),
    };
    let summary = c.read("plan.summary.md").unwrap_or_default();
    let nstages = plan_stages(&summary).len();
    if nstages > 0 {
        let _ = write!(status, " · {nstages} stages");
    }
    if let Some(d) = &diff {
        let count = |k: &str| {
            d.entries
                .iter()
                .filter(|e| crate::spec::fixed_id_len(e.id.as_bytes()) == Some(e.id.len()))
                .filter(|e| e.kind == k)
                .count()
        };
        let (new, changed, violated, dropped) = (
            count("new"),
            count("changed"),
            count("violated"),
            count("dropped"),
        );
        if violated > 0 {
            let _ = write!(status, " · {violated} entries now honoured");
        } else if dropped > 0 {
            let _ = write!(
                status,
                " · {} entries ({new} new, {changed} changed, {dropped} dropped)",
                new + changed + dropped
            );
        } else {
            let _ = write!(
                status,
                " · {} entries ({new} new, {changed} changed)",
                new + changed
            );
        }
    }
    if let Some(cov) = board::last_token(c.conn, c.slug, "cov")? {
        let _ = write!(status, " · coverage {cov}");
    }
    if let Some(review) = board::last_token(c.conn, c.slug, "review")? {
        let _ = write!(status, " · review {review}");
    }
    if let Some(split) = c.field("split")? {
        let list: Vec<String> = split
            .trim_matches(|ch| ch == '[' || ch == ']')
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| format!("#{s}"))
            .collect();
        let _ = write!(status, " · split off: {}", list.join(", "));
    }
    let mut out = format!("{}\n\n{status}\n\n", first_line(issue));
    let body: Vec<&str> = pr.lines().skip(1).collect();
    let body = if body.first() == Some(&"") {
        &body[1..]
    } else {
        &body[..]
    };
    let _ = write!(out, "{}\n\n", body.join("\n"));
    let mut changed = HashSet::new();
    if let Some(d) = &diff {
        if let Some((n, list)) = scenarios(Some(d)) {
            let _ = write!(
                out,
                "## Scenarios covered\nAll {n} scenarios of {}: {list}\n\n",
                issue_ref(c)?
            );
        }
        out.push_str(&requirements(c, Some(d))?);
        let lines: Vec<String> = trim_end(&render(d, Part::Changes, None))
            .lines()
            .map(str::to_owned)
            .collect();
        changed = spec_changes(d, deltas, &lines, &mut out);
    }
    out.push_str(&architecture(c));
    out.push_str(&decisions(c.read("decisions.md").as_deref(), &changed));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ut_stage_refs() {
        assert_eq!(stage_ref("Today s3 restarts it.").as_deref(), Some("s3"));
        assert_eq!(stage_ref("Today `s3` restarts it.").as_deref(), Some("s3"));
        assert_eq!(stage_ref("Today `docs/s3.md` restarts it."), None);
        assert_eq!(stage_ref("x <!-- s1 --> y"), None);
        assert_eq!(stage_ref("```\ns1\n```\n"), None);
        assert_eq!(stage_ref("uses2 and s2x"), None);
        assert_eq!(
            stage_ref("the Fe-Architect said").as_deref(),
            Some("fe-architect")
        );
    }

    #[test]
    fn ut_delta_lines() {
        assert_eq!(
            delta("spec-delta MB-R-1 changed quote=\"a b\" origin=user popup on Esc"),
            Some(Delta {
                id: "MB-R-1".into(),
                kind: "changed".into(),
                origin: "user".into(),
                quote: "a b".into(),
                text: "popup on Esc".into()
            })
        );
    }

    #[test]
    fn ut_nth() {
        assert_eq!(nth(Some("[71, 72]"), 2).as_deref(), Some("72"));
        assert_eq!(nth(Some("[71]"), 2), None);
        assert_eq!(nth(None, 1), None);
    }
}
