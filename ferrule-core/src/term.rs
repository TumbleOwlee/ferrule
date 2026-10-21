//! Markdown and the board's other documents as styled terminal text.
//!
//! Used only when stdout is a terminal or `--format text` asks for it; a
//! pipe gets the markdown unchanged. A soft line break stays a line break:
//! the workflow's files put one paragraph or one item per line, so a single
//! newline is meant. HTML comments (the guide's tags) are dropped; other
//! inline HTML, such as a `<slug>` placeholder, prints as written.

use pulldown_cmark::{Alignment, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use unicode_width::UnicodeWidthChar;

/// How terminal text is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Style {
    /// ANSI colours and attributes; off under `NO_COLOR`.
    pub color: bool,
    /// Columns to wrap paragraphs at.
    pub width: usize,
}

const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const ITALIC: &str = "\x1b[3m";
const UNDERLINE: &str = "\x1b[4m";
const STRIKE: &str = "\x1b[9m";
const RED: &str = "\x1b[31m";
const GREEN: &str = "\x1b[32m";
const YELLOW: &str = "\x1b[33m";
const BLUE: &str = "\x1b[34m";
const MAGENTA: &str = "\x1b[35m";
const CYAN: &str = "\x1b[36m";

impl Style {
    fn paint(self, codes: &str, text: &str) -> String {
        if self.color && !text.is_empty() {
            format!("{codes}{text}{RESET}")
        } else {
            text.to_owned()
        }
    }
}

/// A heading's look by depth below the document's shallowest heading.
fn heading_look(depth: usize) -> String {
    match depth {
        0 => format!("{BOLD}{MAGENTA}"),
        1 => format!("{BOLD}{CYAN}"),
        _ => BOLD.to_owned(),
    }
}

/// Visible columns of `s`, ANSI sequences skipped and wide characters
/// (CJK, emoji) counted as two.
fn width(s: &str) -> usize {
    let mut n = 0;
    let mut esc = false;
    for c in s.chars() {
        if esc {
            esc = c != 'm';
        } else if c == '\x1b' {
            esc = true;
        } else {
            n += c.width().unwrap_or(0);
        }
    }
    n
}

/// `s` without control characters: C0 except newline and tab, DEL, C1 and
/// escape sequences (CSI, OSC, and two-character ones). A carriage return
/// becomes a newline, or nothing before one. Text from a host must not
/// reach the terminal with sequences of its own.
fn sanitize(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\x1b' => match chars.next() {
                // CSI: parameters and intermediates, then a final byte.
                Some('[') => {
                    for d in chars.by_ref() {
                        if ('\x40'..='\x7e').contains(&d) {
                            break;
                        }
                    }
                }
                // OSC, DCS, SOS, PM, APC: a string up to BEL or ST.
                Some(']' | 'P' | 'X' | '^' | '_') => {
                    while let Some(d) = chars.next() {
                        if d == '\x07' || d == '\u{9c}' {
                            break;
                        }
                        if d == '\x1b' {
                            if chars.peek() == Some(&'\\') {
                                chars.next();
                            }
                            break;
                        }
                    }
                }
                _ => {}
            },
            '\r' => {
                if chars.peek() != Some(&'\n') {
                    out.push('\n');
                }
            }
            '\n' | '\t' => out.push(c),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

/// Splits `line` at spaces into pieces of at most `max` visible columns; a
/// word longer than `max` stays whole.
fn wrap(line: &str, max: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for word in line.split(' ') {
        if !cur.is_empty() && width(&cur) + 1 + width(word) > max {
            out.push(std::mem::take(&mut cur));
        } else if !cur.is_empty() {
            cur.push(' ');
        }
        cur.push_str(word);
    }
    out.push(cur);
    out
}

struct Item {
    /// Column the item's text starts at.
    col: usize,
    /// The marker, until the item's first line takes it.
    marker: Option<String>,
}

struct Table {
    align: Vec<Alignment>,
    rows: Vec<Vec<String>>,
    head_rows: usize,
}

#[derive(PartialEq, Eq)]
enum Last {
    None,
    Heading,
    Paragraph,
    Other,
}

struct Renderer {
    style: Style,
    out: Vec<String>,
    inline: String,
    /// Open inline styles, re-applied after an inner one closes.
    marks: Vec<String>,
    /// Column of text under the current heading.
    base: usize,
    min_level: usize,
    items: Vec<Item>,
    /// `None` for a bullet list, else the next number.
    lists: Vec<Option<u64>>,
    code: Option<String>,
    html: Option<String>,
    table: Option<Table>,
    links: Vec<(String, usize)>,
    last: Last,
}

fn level(l: HeadingLevel) -> usize {
    l as usize
}

impl Renderer {
    fn col(&self) -> usize {
        self.items.last().map_or(self.base, |i| i.col)
    }

    fn push_mark(&mut self, codes: &str) {
        self.marks.push(codes.to_owned());
        if self.style.color {
            self.inline.push_str(codes);
        }
    }

    fn pop_mark(&mut self) {
        self.marks.pop();
        if self.style.color {
            self.inline.push_str(RESET);
            for m in &self.marks {
                self.inline.push_str(m);
            }
        }
    }

    /// A blank line before a new block, unless it opens a list item or
    /// attaches a list to the paragraph above it.
    fn gap(&mut self, list: bool) {
        let first_in_item = self.items.last().is_some_and(|i| i.marker.is_some());
        let attach = list && (self.last == Last::Paragraph || !self.lists.is_empty());
        if !first_in_item
            && !attach
            && self.last != Last::Heading
            && self.out.last().is_some_and(|l| !l.is_empty())
        {
            self.out.push(String::new());
        }
    }

    /// The prefix of the next line at `col`: the item's marker when it is
    /// still pending, else spaces.
    fn prefix(&mut self, col: usize) -> String {
        match self.items.last_mut().and_then(|i| i.marker.take()) {
            Some(marker) => {
                let at = col.saturating_sub(width(&marker));
                format!("{}{marker}", " ".repeat(at))
            }
            None => " ".repeat(col),
        }
    }

    fn flush(&mut self) {
        let text = std::mem::take(&mut self.inline);
        if text.is_empty() {
            return;
        }
        let col = self.col();
        let max = self.style.width.saturating_sub(col).max(20);
        for logical in text.split('\n') {
            for piece in wrap(logical, max) {
                let p = self.prefix(col);
                self.out.push(format!("{p}{piece}").trim_end().to_owned());
            }
        }
    }

    fn start(&mut self, tag: Tag) {
        match tag {
            Tag::Paragraph => {
                self.flush();
                self.gap(false);
            }
            Tag::Heading { .. } => {
                self.flush();
                self.items.clear();
                self.lists.clear();
                self.gap(false);
            }
            Tag::List(first) => {
                self.flush();
                self.gap(true);
                self.lists.push(first);
            }
            Tag::Item => {
                self.flush();
                let at = self.col();
                let marker = if let Some(Some(n)) = self.lists.last_mut() {
                    let m = format!("{n}. ");
                    *n += 1;
                    m
                } else {
                    let bullet = ["• ", "◦ ", "▪ "][(self.lists.len() + 2) % 3];
                    self.style.paint(DIM, bullet)
                };
                let col = at + width(&marker);
                self.items.push(Item {
                    col,
                    marker: Some(marker),
                });
            }
            Tag::CodeBlock(_) => {
                self.flush();
                self.gap(false);
                self.code = Some(String::new());
            }
            Tag::HtmlBlock => {
                self.flush();
                self.html = Some(String::new());
            }
            Tag::BlockQuote(_) => {
                self.flush();
                self.gap(false);
                self.items.push(Item {
                    col: self.col() + 2,
                    marker: Some(self.style.paint(DIM, "│ ")),
                });
            }
            Tag::Table(align) => {
                self.flush();
                self.gap(false);
                self.table = Some(Table {
                    align,
                    rows: Vec::new(),
                    head_rows: 0,
                });
            }
            Tag::TableHead | Tag::TableRow => {
                if let Some(t) = &mut self.table {
                    t.rows.push(Vec::new());
                }
            }
            Tag::TableCell => self.inline.clear(),
            Tag::Emphasis => self.push_mark(ITALIC),
            Tag::Strong => self.push_mark(BOLD),
            Tag::Strikethrough => self.push_mark(STRIKE),
            Tag::Link { dest_url, .. } => {
                self.links.push((dest_url.to_string(), self.inline.len()));
                self.push_mark(&format!("{UNDERLINE}{BLUE}"));
            }
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => {
                self.flush();
                self.last = Last::Paragraph;
            }
            TagEnd::Heading(l) => {
                let depth = level(l).saturating_sub(self.min_level);
                let text = std::mem::take(&mut self.inline).replace('\n', " ");
                let look = heading_look(depth);
                let line = format!(
                    "{}{}",
                    " ".repeat(2 * depth),
                    self.style.paint(&look, &text)
                );
                self.out.push(line);
                self.base = 2 * (depth + 1);
                self.last = Last::Heading;
            }
            TagEnd::List(_) => {
                self.flush();
                self.lists.pop();
                self.last = Last::Other;
            }
            TagEnd::Item | TagEnd::BlockQuote(_) => {
                self.flush();
                if let Some(item) = self.items.pop()
                    && let Some(marker) = item.marker
                {
                    let p = " ".repeat(item.col.saturating_sub(width(&marker)));
                    self.out.push(format!("{p}{marker}").trim_end().to_owned());
                }
                self.last = Last::Other;
            }
            TagEnd::CodeBlock => {
                let code = self.code.take().unwrap_or_default();
                let col = self.col() + 2;
                for line in code.strip_suffix('\n').unwrap_or(&code).split('\n') {
                    let p = self.prefix(col);
                    self.out
                        .push(format!("{p}{}", self.style.paint(GREEN, line)));
                }
                self.last = Last::Other;
            }
            TagEnd::HtmlBlock => {
                let html = self.html.take().unwrap_or_default();
                let kept: Vec<&str> = html
                    .lines()
                    .filter(|l| !l.trim_start().starts_with("<!--"))
                    .collect();
                if !kept.is_empty() {
                    self.gap(false);
                    let col = self.col();
                    for l in kept {
                        self.out.push(format!("{}{l}", " ".repeat(col)));
                    }
                    self.last = Last::Other;
                }
            }
            TagEnd::Table => self.table_end(),
            TagEnd::TableHead => {
                if let Some(t) = &mut self.table {
                    t.head_rows = t.rows.len();
                }
            }
            TagEnd::TableCell => {
                let cell = std::mem::take(&mut self.inline).replace('\n', " ");
                if let Some(row) = self.table.as_mut().and_then(|t| t.rows.last_mut()) {
                    row.push(cell);
                }
            }
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough => self.pop_mark(),
            TagEnd::Link => {
                self.pop_mark();
                if let Some((url, at)) = self.links.pop() {
                    let text = self.inline.get(at..).unwrap_or("").to_owned();
                    if !url.is_empty() && !url.starts_with('#') && !text.contains(url.as_str()) {
                        let shown = self.style.paint(DIM, &format!("({url})"));
                        self.inline.push(' ');
                        self.inline.push_str(&shown);
                    }
                }
            }
            _ => {}
        }
    }

    fn table_end(&mut self) {
        let Some(t) = self.table.take() else { return };
        let cols = t.rows.iter().map(Vec::len).max().unwrap_or(0);
        let widths: Vec<usize> = (0..cols)
            .map(|c| {
                t.rows
                    .iter()
                    .filter_map(|r| r.get(c))
                    .map(|s| width(s))
                    .max()
                    .unwrap_or(0)
            })
            .collect();
        let col = self.col();
        let total: usize = widths.iter().sum::<usize>() + 2 * cols.saturating_sub(1);
        if cols > 1 && col + total > self.style.width {
            self.records(&t, col);
            self.last = Last::Other;
            return;
        }
        for (i, row) in t.rows.iter().enumerate() {
            let cells: Vec<String> = (0..cols)
                .map(|c| {
                    let cell = row.get(c).map_or("", String::as_str);
                    let pad = " ".repeat(widths[c] - width(cell));
                    let cell = if i < t.head_rows {
                        self.style.paint(BOLD, cell)
                    } else {
                        cell.to_owned()
                    };
                    match t.align.get(c) {
                        Some(Alignment::Right) => format!("{pad}{cell}"),
                        _ => format!("{cell}{pad}"),
                    }
                })
                .collect();
            let line = format!("{}{}", " ".repeat(col), cells.join("  "));
            self.out.push(line.trim_end().to_owned());
            if i + 1 == t.head_rows {
                let rule: Vec<String> = widths.iter().map(|w| "─".repeat(*w)).collect();
                let rule = self.style.paint(DIM, &rule.join("  "));
                self.out.push(format!("{}{rule}", " ".repeat(col)));
            }
        }
        self.last = Last::Other;
    }

    /// A table too wide for the terminal: per body row, its first cell as
    /// a title, then each other cell under its column's header.
    fn records(&mut self, t: &Table, col: usize) {
        let head = t.rows.first().filter(|_| t.head_rows > 0);
        let label = |c: usize| head.and_then(|h| h.get(c)).cloned().unwrap_or_default();
        let cols = t.rows.iter().map(Vec::len).max().unwrap_or(0);
        let lw = (1..cols).map(|c| width(&label(c))).max().unwrap_or(0);
        let at = col + 2 + lw + 2;
        let max = self.style.width.saturating_sub(at).max(20);
        for (n, row) in t.rows.iter().skip(t.head_rows).enumerate() {
            if n > 0 {
                self.out.push(String::new());
            }
            let title = row.first().map_or("", String::as_str);
            self.out.push(format!(
                "{}{}",
                " ".repeat(col),
                self.style.paint(BOLD, title)
            ));
            for c in 1..cols {
                let l = label(c);
                let pad = " ".repeat(lw - width(&l));
                let cell = row.get(c).map_or("", String::as_str);
                for (k, piece) in wrap(cell, max).into_iter().enumerate() {
                    let lead = if k == 0 {
                        format!(
                            "{}{}{pad}  ",
                            " ".repeat(col + 2),
                            self.style.paint(DIM, &l)
                        )
                    } else {
                        " ".repeat(at)
                    };
                    self.out
                        .push(format!("{lead}{piece}").trim_end().to_owned());
                }
            }
        }
    }

    fn event(&mut self, e: Event) {
        match e {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(t) => {
                if let Some(code) = &mut self.code {
                    code.push_str(&t);
                } else {
                    self.inline.push_str(&t);
                }
            }
            Event::Code(t) => {
                if self.style.color {
                    self.inline.push_str(YELLOW);
                    self.inline.push_str(&t);
                    self.inline.push_str(RESET);
                    for m in &self.marks {
                        self.inline.push_str(m);
                    }
                } else {
                    self.inline.push('`');
                    self.inline.push_str(&t);
                    self.inline.push('`');
                }
            }
            Event::Html(t) => {
                if let Some(h) = &mut self.html {
                    h.push_str(&t);
                }
            }
            Event::InlineHtml(t) if !t.starts_with("<!--") => self.inline.push_str(&t),
            Event::SoftBreak | Event::HardBreak => self.inline.push('\n'),
            Event::Rule => {
                self.flush();
                self.gap(false);
                let col = self.col();
                let rule = "─".repeat(self.style.width.saturating_sub(col).min(60));
                let line = format!("{}{}", " ".repeat(col), self.style.paint(DIM, &rule));
                self.out.push(line);
                self.last = Last::Other;
            }
            Event::TaskListMarker(done) => {
                self.inline.push_str(if done { "☑ " } else { "☐ " });
            }
            _ => {}
        }
    }
}

/// `md` as terminal text.
#[must_use]
pub fn markdown(md: &str, style: Style) -> String {
    let opts = Options::ENABLE_TABLES | Options::ENABLE_TASKLISTS | Options::ENABLE_STRIKETHROUGH;
    let md = sanitize(md);
    let events: Vec<Event> = Parser::new_ext(&md, opts).collect();
    let min_level = events
        .iter()
        .filter_map(|e| match e {
            Event::Start(Tag::Heading { level: l, .. }) => Some(level(*l)),
            _ => None,
        })
        .min()
        .unwrap_or(1);
    let mut r = Renderer {
        style,
        out: Vec::new(),
        inline: String::new(),
        marks: Vec::new(),
        base: 0,
        min_level,
        items: Vec::new(),
        lists: Vec::new(),
        code: None,
        html: None,
        table: None,
        links: Vec::new(),
        last: Last::None,
    };
    for e in events {
        r.event(e);
    }
    r.flush();
    while r.out.last().is_some_and(String::is_empty) {
        r.out.pop();
    }
    let mut text = r.out.join("\n");
    if !text.is_empty() {
        text.push('\n');
    }
    text
}

/// Headings, `(level, text)` in file order, as a tree: each heading under
/// the closest one above it of a shallower level.
#[must_use]
pub fn outline(headings: &[(usize, &str)], style: Style) -> String {
    // parent[i]: the closest earlier heading of a shallower level.
    let parent: Vec<Option<usize>> = (0..headings.len())
        .map(|i| (0..i).rev().find(|&j| headings[j].0 < headings[i].0))
        .collect();
    let last_child = |i: usize| !(i + 1..headings.len()).any(|j| parent[j] == parent[i]);
    let mut out = String::new();
    for (i, (_, text)) in headings.iter().enumerate() {
        let mut chain = Vec::new();
        let mut at = parent[i];
        while let Some(p) = at {
            chain.push(p);
            at = parent[p];
        }
        chain.reverse();
        let mut line = String::new();
        // Ancestors below the root: a bar while they have siblings to come.
        for &a in chain.iter().skip(1) {
            line.push_str(if last_child(a) { "   " } else { "│  " });
        }
        let line = if chain.is_empty() {
            style.paint(BOLD, text)
        } else {
            let branch = if last_child(i) { "└─ " } else { "├─ " };
            format!("{}{text}", style.paint(DIM, &format!("{line}{branch}")))
        };
        out.push_str(&line);
        out.push('\n');
    }
    out
}

/// A board state's colour.
#[must_use]
pub fn state(state: &str, style: Style) -> String {
    let look = match state {
        "open" => BLUE,
        "inprogress" => YELLOW,
        "inreview" => MAGENTA,
        "done" => GREEN,
        _ => RED,
    };
    style.paint(look, state)
}

/// `s` on one line of at most `max` visible columns, control characters
/// dropped; `…` ends it when something was cut.
#[must_use]
pub fn cut(s: &str, max: usize) -> String {
    let clean: String = s.chars().filter(|c| !c.is_control()).collect();
    if width(&clean) <= max {
        return clean;
    }
    let mut out = String::new();
    let mut used = 0;
    for c in clean.chars() {
        let w = c.width().unwrap_or(0);
        if used + w + 1 > max {
            break;
        }
        used += w;
        out.push(c);
    }
    out.push('…');
    out
}

/// `text` bold.
#[must_use]
pub fn strong(text: &str, style: Style) -> String {
    style.paint(BOLD, text)
}

/// `text` dimmed.
#[must_use]
pub fn faint(text: &str, style: Style) -> String {
    style.paint(DIM, text)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLAIN: Style = Style {
        color: false,
        width: 40,
    };

    #[test]
    fn ut_headings_indent_their_sections() {
        let md = "## Gate 2\n\nSpawn the planner.\n\n### On approval\n\n1. Files the issue\n2. Adds the worktree\n";
        assert_eq!(
            markdown(md, PLAIN),
            "Gate 2\n  Spawn the planner.\n\n  On approval\n    1. Files the issue\n    2. Adds the worktree\n"
        );
    }

    #[test]
    fn ut_soft_breaks_stay_and_lists_attach() {
        let md = "Stops:\n- s1 — yes\n  - why\n- s2\nNext: go\n";
        assert_eq!(
            markdown(md, PLAIN),
            "Stops:\n• s1 — yes\n  ◦ why\n• s2\n  Next: go\n"
        );
    }

    #[test]
    fn ut_inline_marks_and_comments() {
        let md = "<!-- lane: fast -->\nRun `ferrule gate next` for **<slug>**, see [docs](https://x.y).\n";
        assert_eq!(
            markdown(md, PLAIN),
            "Run `ferrule gate next` for <slug>, see\ndocs (https://x.y).\n"
        );
        let color = Style {
            color: true,
            width: 80,
        };
        assert_eq!(
            markdown("a **b `c` d** e\n", color),
            "a \x1b[1mb \x1b[33mc\x1b[0m\x1b[1m d\x1b[0m e\n"
        );
    }

    #[test]
    fn ut_wraps_with_a_hanging_indent() {
        let md = "- one two three four five six seven eight nine ten eleven\n";
        assert_eq!(
            markdown(md, PLAIN),
            "• one two three four five six seven\n  eight nine ten eleven\n"
        );
        assert_eq!(width("\x1b[1mab\x1b[0m"), 2);
    }

    #[test]
    fn ut_tables_align_and_code_blocks_indent() {
        let md = "| Stage | Title |\n|---|---|\n| s1 | Backoff |\n| s10 | UI |\n\n```sh\ncargo test\n```\n";
        assert_eq!(
            markdown(md, PLAIN),
            "Stage  Title\n─────  ───────\ns1     Backoff\ns10    UI\n\n  cargo test\n"
        );
    }

    #[test]
    fn ut_wide_characters_align_in_tables() {
        let md = "| Name | Note |\n|---|---|\n| 日本語 | ok |\n| ab | 🚀 go |\n";
        assert_eq!(
            markdown(md, PLAIN),
            "Name    Note\n──────  ─────\n日本語  ok\nab      🚀 go\n"
        );
        assert_eq!(width("日本"), 4);
    }

    #[test]
    fn ut_escape_sequences_in_input_are_removed() {
        let color = Style {
            color: true,
            width: 80,
        };
        let md = "a\x1b[31mred\x1b[0m \x1b]0;title\x07b\x07\x7f\u{9b}c\r\nd\te\n";
        assert_eq!(markdown(md, color), "ared bc\nd\te\n");
        assert_eq!(markdown("x\x1b]8;;u\x1b\\y\n", color), "xy\n");
    }

    #[test]
    fn ut_a_table_too_wide_becomes_records() {
        let md = "| | fast | full |\n|---|---|---|\n| Brief | no stop of its own, approved at the architecture stop | own stop |\n";
        assert_eq!(
            markdown(md, PLAIN),
            "Brief\n  fast  no stop of its own, approved at\n        the architecture stop\n  full  own stop\n"
        );
    }

    #[test]
    fn ut_cut_ends_in_an_ellipsis() {
        assert_eq!(cut("abcdef", 6), "abcdef");
        assert_eq!(cut("abcdefg", 6), "abcde…");
        assert_eq!(cut("日本語", 4), "日…");
        assert_eq!(cut("a\x1b[1mb", 9), "a[1mb");
    }

    #[test]
    fn ut_outline_draws_a_tree() {
        let h = [
            (1, "Title"),
            (2, "Alpha"),
            (3, "Alpha child"),
            (3, "Alpha other"),
            (2, "Beta"),
            (1, "Second"),
        ];
        assert_eq!(
            outline(&h, PLAIN),
            "Title\n├─ Alpha\n│  ├─ Alpha child\n│  └─ Alpha other\n└─ Beta\nSecond\n"
        );
    }
}
