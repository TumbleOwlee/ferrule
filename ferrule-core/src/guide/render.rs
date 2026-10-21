//! A guide topic rendered for one run: `{slug}`-style variables filled
//! from the [`Context`], and the blocks tagged for another lane or stack
//! dropped.

use std::fmt::Write as _;

use crate::markdown;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lane {
    Fast,
    Full,
}

impl Lane {
    #[must_use]
    pub fn parse(s: &str) -> Option<Lane> {
        match s {
            "fast" => Some(Lane::Fast),
            "full" => Some(Lane::Full),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Lane::Fast => "fast",
            Lane::Full => "full",
        }
    }
}

/// What the guide knows about the run it is printed for.
#[derive(Debug, Clone, Default)]
pub struct Context {
    /// None prints both lanes' blocks, each labelled.
    pub lane: Option<Lane>,
    /// The parent card's fields and log tokens, `key=value` each.
    pub done: Vec<String>,
    /// Placeholder values: `slug`, `stage`, `card`, `run_dir`, `worktree`,
    /// `branch`, `issue`, `pr`, `main`.
    pub vars: Vec<(&'static str, String)>,
}

impl Context {
    /// Whether the card carries `token`: `key=value` as a field or a log
    /// token, a bare `key` as a non-empty field or any `key=` log token.
    pub(super) fn has(&self, token: &str) -> bool {
        if token.contains('=') {
            self.done.iter().any(|t| t == token)
        } else {
            let prefix = format!("{token}=");
            self.done
                .iter()
                .any(|t| t.starts_with(&prefix) && t.len() > prefix.len())
        }
    }
}

/// The placeholders [`render`] fills; any other `{name}` is left alone.
pub const VARS: &[&str] = &[
    "slug", "stage", "card", "run_dir", "worktree", "branch", "issue", "pr", "main",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Tag {
    Lane(Lane),
    Done(String),
}

/// A tag comment line, or None for any other line. A malformed one is an
/// error naming the line.
pub(super) fn tag(line: &str) -> Option<Result<Tag, String>> {
    let inner = line
        .trim()
        .strip_prefix("<!--")?
        .strip_suffix("-->")?
        .trim();
    let (key, value) = inner.split_once(':')?;
    let value = value.trim();
    match key.trim() {
        "lane" => Some(
            Lane::parse(value)
                .map(Tag::Lane)
                .ok_or_else(|| line.to_owned()),
        ),
        "done" if !value.is_empty() && !value.contains(' ') => {
            Some(Ok(Tag::Done(value.to_owned())))
        }
        "done" => Some(Err(line.to_owned())),
        _ => None,
    }
}

fn heading_level(line: &str) -> Option<usize> {
    markdown::headings(line).first().map(|h| h.level)
}

/// One block: its tags and its lines; blank lines separate blocks outside
/// fenced code.
struct Block<'a> {
    tags: Vec<Tag>,
    lines: Vec<&'a str>,
}

fn blocks(text: &str) -> Vec<Block<'_>> {
    let mut out = Vec::new();
    let mut tags = Vec::new();
    let mut lines: Vec<&str> = Vec::new();
    for l in markdown::classified(text) {
        let (line, code) = (l.text, l.fenced || l.delimiter);
        if !code
            && lines.is_empty()
            && let Some(Ok(t)) = tag(line)
        {
            tags.push(t);
            continue;
        }
        if !code && line.trim().is_empty() {
            if !lines.is_empty() {
                out.push(Block {
                    tags: std::mem::take(&mut tags),
                    lines: std::mem::take(&mut lines),
                });
            }
            continue;
        }
        lines.push(line);
    }
    if !lines.is_empty() {
        out.push(Block { tags, lines });
    }
    out
}

fn fill(line: &str, ctx: &Context) -> String {
    let mut out = line.to_owned();
    for name in VARS {
        let key = format!("{{{name}}}");
        if !out.contains(&key) {
            continue;
        }
        let value = ctx
            .vars
            .iter()
            .find(|(k, _)| k == name)
            .map_or_else(|| format!("<{name}>"), |(_, v)| v.clone());
        out = out.replace(&key, &value);
    }
    out
}

/// `text` cut to `ctx`: blocks of the other lane dropped, done blocks
/// marked, placeholders filled.
#[must_use]
pub fn render(text: &str, ctx: &Context) -> String {
    let mut out = String::new();
    // Tags inherited from enclosing headings: (level, tags).
    let mut scopes: Vec<(usize, Vec<Tag>)> = Vec::new();
    for block in blocks(text) {
        let level = heading_level(block.lines[0]);
        if let Some(level) = level {
            scopes.retain(|(l, _)| *l < level);
        }
        let mut tags: Vec<Tag> = scopes.iter().flat_map(|(_, t)| t.clone()).collect();
        tags.extend(block.tags.iter().cloned());
        if let Some(level) = level {
            scopes.push((level, block.tags.clone()));
        }
        let lane = tags.iter().find_map(|t| match t {
            Tag::Lane(l) => Some(*l),
            Tag::Done(_) => None,
        });
        if let (Some(want), Some(have)) = (lane, ctx.lane)
            && want != have
        {
            continue;
        }
        let own_done = block
            .tags
            .iter()
            .any(|t| matches!(t, Tag::Done(d) if ctx.has(d)));
        let mut marks = Vec::new();
        if ctx.lane.is_none()
            && let Some(l) = block.tags.iter().find_map(|t| match t {
                Tag::Lane(l) => Some(*l),
                Tag::Done(_) => None,
            })
        {
            marks.push(format!("{} lane", l.as_str()));
        }
        if own_done {
            marks.push("done".to_owned());
        }
        if !out.is_empty() {
            out.push('\n');
        }
        let mut lines = block.lines.iter();
        if !marks.is_empty() {
            let mark = marks.join(", ");
            if level.is_some() {
                let first = lines.next().map(|l| fill(l, ctx)).unwrap_or_default();
                let _ = writeln!(out, "{first} ({mark})");
            } else {
                let _ = writeln!(out, "({mark})");
            }
        }
        for line in lines {
            out.push_str(&fill(line, ctx));
            out.push('\n');
        }
    }
    out
}
