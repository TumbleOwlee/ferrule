//! A run's `spec-diff.md`: the brief's scenarios, its spec entries under
//! local (`R1`, `E1`, `S1`) or fixed IDs, other spec changes, deferred
//! points and the dated revisions, plus the markdown the issue and PR bodies
//! build from it.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

use crate::markdown;
use crate::spec::fixed_id_len;

/// Which block of the spec diff to render.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    /// `## Scenarios`, each with its Given/When/Then steps.
    Scenarios,
    /// `## Requirements` as a list, local IDs written back where known.
    Requirements,
    /// `### Requirements` table and `### Other spec changes` table.
    Entries,
    /// The dated revisions, one line per item with who decided it.
    Changes,
    /// `### Deferred` list.
    Deferred,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scenario {
    pub id: String,
    pub title: String,
    /// The `Given` / `When` / `Then` / `And` lines, verbatim.
    pub steps: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub id: String,
    /// The word after `—` in the heading (`new`, `changed`, `dropped`,
    /// `violated`, …); `new` when the heading has none.
    pub kind: String,
    /// `Change:` lines, each a note on the kind.
    pub changes: Vec<String>,
    /// The requirement's text lines.
    pub text: Vec<String>,
    /// The `- ` lines: why an entry is dropped or violated.
    pub why: Vec<String>,
}

/// One dated revision line: its `; `-separated items and who decided them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Revision {
    pub items: Vec<String>,
    pub who: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SpecDiff {
    /// Fixed ID → the local ID it had before gate 2 (`· was R1`).
    pub was: HashMap<String, String>,
    pub scenarios: Vec<Scenario>,
    pub entries: Vec<Entry>,
    pub revisions: Vec<Revision>,
    /// `## Other spec changes` items without their `- `.
    pub other: Vec<String>,
    /// `## Deferred` lines, verbatim.
    pub deferred: Vec<String>,
}

/// The `n`th blank-separated word of a line (0-based); a heading keeps its
/// ID in word 1.
fn word(line: &str, n: usize) -> &str {
    line.split([' ', '\t'])
        .filter(|w| !w.is_empty())
        .nth(n)
        .unwrap_or("")
}

fn is_scenario_heading(line: &str) -> Option<&str> {
    let rest = line.strip_prefix("## S")?;
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    (digits > 0)
        .then(|| rest[digits..].strip_prefix(" — "))
        .flatten()
}

/// `· was R1` / `· was E2` at the end of an entry heading.
fn was_suffix(line: &str) -> Option<&str> {
    let (_, tail) = line.rsplit_once(" · was ")?;
    let mut chars = tail.chars();
    let shape = matches!(chars.next(), Some('R' | 'E'))
        && !tail[1..].is_empty()
        && tail[1..].bytes().all(|b| b.is_ascii_digit());
    shape.then_some(tail)
}

/// `- rev <n> (<date>) — <items> (<who>)`, with `who` one of user, review,
/// impl; other revision lines carry no decision and are skipped.
fn revision(line: &str) -> Option<Revision> {
    let rest = line.strip_prefix("- rev ")?;
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    let rest = rest.get(digits..)?.strip_prefix(" (")?;
    if digits == 0 {
        return None;
    }
    let date = rest
        .bytes()
        .take_while(|b| b.is_ascii_digit() || *b == b'-')
        .count();
    let body = rest.get(date..)?.strip_prefix(") — ")?;
    if date == 0 {
        return None;
    }
    let (body, who) = ["user", "review", "impl"]
        .iter()
        .find_map(|who| Some((body.strip_suffix(&format!(" ({who})"))?, *who)))?;
    Some(Revision {
        items: body.split("; ").map(str::to_owned).collect(),
        who: who.to_owned(),
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    None,
    Summary,
    Other,
    Deferred,
    Scenario,
    Entry,
}

impl SpecDiff {
    #[must_use]
    pub fn parse(text: &str) -> Self {
        let mut diff = SpecDiff::default();
        let mut sec = Section::None;
        for line in markdown::classified(text) {
            if line.fenced {
                // Code in an entry stays in its text, delimiters included.
                if sec == Section::Entry
                    && let Some(entry) = diff.entries.last_mut()
                {
                    entry.text.push(line.text.to_owned());
                }
                continue;
            }
            let line = line.text;
            if line.starts_with("## ")
                && let Some(local) = was_suffix(line)
            {
                diff.was.insert(word(line, 1).to_owned(), local.to_owned());
            }
            match line {
                "## Summary" => sec = Section::Summary,
                "## Other spec changes" => sec = Section::Other,
                "## Deferred" => sec = Section::Deferred,
                _ if let Some(title) = is_scenario_heading(line) => {
                    sec = Section::Scenario;
                    diff.scenarios.push(Scenario {
                        id: word(line, 1).to_owned(),
                        title: title.to_owned(),
                        steps: Vec::new(),
                    });
                }
                _ if sec == Section::Scenario
                    && ["Given ", "When ", "Then ", "And "]
                        .iter()
                        .any(|w| line.starts_with(w)) =>
                {
                    if let Some(s) = diff.scenarios.last_mut() {
                        s.steps.push(line.to_owned());
                    }
                }
                _ if sec == Section::Deferred && line.starts_with("- ") => {
                    diff.deferred.push(line.to_owned());
                }
                _ if line.starts_with("## ") => {
                    sec = Section::Entry;
                    let kind = if word(line, 2) == "—" && !word(line, 3).is_empty() {
                        word(line, 3)
                    } else {
                        "new"
                    };
                    diff.entries.push(Entry {
                        id: word(line, 1).to_owned(),
                        kind: kind.to_owned(),
                        changes: Vec::new(),
                        text: Vec::new(),
                        why: Vec::new(),
                    });
                }
                _ if sec == Section::Entry => {
                    let Some(entry) = diff.entries.last_mut() else {
                        continue;
                    };
                    if let Some(change) = line.strip_prefix("Change: ") {
                        entry.changes.push(change.to_owned());
                    } else if let Some(why) = line.strip_prefix("- ") {
                        entry.why.push(why.to_owned());
                    } else if !line.is_empty() && !line.starts_with(' ') {
                        entry.text.push(line.to_owned());
                    }
                }
                _ if sec == Section::Summary => {
                    if let Some(rev) = revision(line) {
                        diff.revisions.push(rev);
                    }
                }
                _ if sec == Section::Other => {
                    if let Some(item) = line.strip_prefix("- ") {
                        diff.other.push(item.to_owned());
                    }
                }
                _ => {}
            }
        }
        diff
    }

    /// `s` with every fixed ID the diff knows a local ID for written back
    /// as that local ID.
    fn back(&self, s: &str) -> String {
        let bytes = s.as_bytes();
        let mut out = String::new();
        let mut copied = 0;
        let mut i = 0;
        while i < bytes.len() {
            if let Some(len) = fixed_id_len(&bytes[i..]) {
                let id = &s[i..i + len];
                out.push_str(&s[copied..i]);
                out.push_str(self.was.get(id).map_or(id, String::as_str));
                i += len;
                copied = i;
            } else {
                i += 1;
            }
        }
        out.push_str(&s[copied..]);
        out
    }

    /// The local ID shown for `id`: its `was` ID when it has one.
    fn shown<'a>(&'a self, id: &'a str) -> &'a str {
        self.was.get(id).map_or(id, String::as_str)
    }
}

/// Options for [`render`].
#[derive(Debug, Clone, Default)]
pub struct Render {
    /// Keep only the scenarios and entries named, by local or fixed ID.
    pub only: Option<Vec<String>>,
    /// With `only`: keep "Other spec changes" rows that cite no ID at all.
    pub first: bool,
    /// PR form of the entries table: an `Issue` column with each entry's
    /// local ID, and these IDs marked as reworded after filing.
    pub issue_ids: Option<HashSet<String>>,
}

impl Render {
    fn keep(&self, diff: &SpecDiff, id: &str) -> bool {
        let Some(only) = &self.only else { return true };
        only.iter().any(|o| o == id) || diff.was.get(id).is_some_and(|w| only.contains(w))
    }

    /// Whether an "Other spec changes" item stays under `only`: it cites one
    /// of the IDs, or (with `first`) cites no ID at all.
    fn cites(&self, item: &str) -> bool {
        let Some(only) = &self.only else { return true };
        if !mentions_any_id(item) {
            return self.first;
        }
        only.iter().any(|id| mentions(item, id))
    }
}

fn is_alnum(b: u8) -> bool {
    b.is_ascii_alphanumeric()
}

/// Whether `s` holds something shaped like an ID (`R1`, `E-2`, `MB-R-042`)
/// standing on its own.
fn mentions_any_id(s: &str) -> bool {
    let b = s.as_bytes();
    (0..b.len()).any(|start| {
        if start > 0 && is_alnum(b[start - 1]) {
            return false;
        }
        let mut i = start;
        if b.len() >= i + 3
            && b[i].is_ascii_uppercase()
            && b[i + 1].is_ascii_uppercase()
            && b[i + 2] == b'-'
            && id_tail(b, i + 3).is_some()
        {
            i += 3;
        }
        id_tail(b, i).is_some()
    })
}

/// `R` or `E`, an optional `-`, digits, then a non-alphanumeric or the end:
/// the end offset.
fn id_tail(b: &[u8], mut i: usize) -> Option<usize> {
    if !matches!(b.get(i), Some(b'R' | b'E')) {
        return None;
    }
    i += 1;
    if b.get(i) == Some(&b'-') {
        i += 1;
    }
    let digits = b[i.min(b.len())..]
        .iter()
        .take_while(|c| c.is_ascii_digit())
        .count();
    if digits == 0 {
        return None;
    }
    i += digits;
    match b.get(i) {
        Some(&c) if is_alnum(c) => None,
        _ => Some(i),
    }
}

/// Whether `id` stands on its own in `s`: not preceded by an alphanumeric or
/// `-`, not followed by an alphanumeric.
fn mentions(s: &str, id: &str) -> bool {
    s.match_indices(id).any(|(at, _)| {
        let b = s.as_bytes();
        let before = at.checked_sub(1).map(|i| b[i]);
        let after = b.get(at + id.len()).copied();
        !before.is_some_and(|c| is_alnum(c) || c == b'-') && !after.is_some_and(is_alnum)
    })
}

/// A table cell: pipes escaped, surrounding blanks trimmed, `—` when empty.
fn cell(s: &str) -> String {
    let s = s.replace('|', "\\|");
    let s = s.trim_matches([' ', '\t']);
    if s.is_empty() {
        "—".to_owned()
    } else {
        s.to_owned()
    }
}

/// The rendered part: its heading, its block and one trailing blank line,
/// or nothing when the part is empty.
#[must_use]
pub fn render(diff: &SpecDiff, part: Part, opts: &Render) -> String {
    match part {
        Part::Scenarios => scenarios(diff, opts),
        Part::Requirements => requirements(diff, opts),
        Part::Entries => entries(diff, opts) + &other_changes(diff, opts),
        Part::Changes => changes(diff),
        Part::Deferred => deferred(diff),
    }
}

fn scenarios(diff: &SpecDiff, opts: &Render) -> String {
    let mut block = String::new();
    for s in diff.scenarios.iter().filter(|s| opts.keep(diff, &s.id)) {
        let _ = writeln!(block, "- **{}** — {}", s.id, diff.back(&s.title));
        for step in &s.steps {
            let _ = writeln!(block, "  - {}", diff.back(step));
        }
    }
    if block.is_empty() {
        return block;
    }
    format!("## Scenarios\n{block}\n")
}

fn requirements(diff: &SpecDiff, opts: &Render) -> String {
    let mut block = String::new();
    let mut violated = 0;
    for e in diff.entries.iter().filter(|e| opts.keep(diff, &e.id)) {
        let text = if e.text.is_empty() { &e.why } else { &e.text };
        let line = match e.kind.as_str() {
            "dropped" => format!("(dropped) {}", e.why.join("; ")),
            "changed" => format!("(changed) {}", spaced(text, &e.why)),
            _ => spaced(text, &e.why),
        };
        if e.kind == "violated" {
            violated += 1;
        }
        let _ = writeln!(block, "- **{}** {}", diff.shown(&e.id), diff.back(&line));
    }
    if block.is_empty() {
        return block;
    }
    let lead = if violated > 0 {
        "No spec change: the code violates the requirements below.\n\n"
    } else {
        ""
    };
    format!("## Requirements\n{lead}{block}\n")
}

fn entries(diff: &SpecDiff, opts: &Render) -> String {
    let mut rows = String::new();
    for e in diff.entries.iter().filter(|e| opts.keep(diff, &e.id)) {
        let mut kind = e.kind.clone();
        for c in &e.changes {
            let _ = write!(kind, " · {c}");
        }
        let text = if e.text.is_empty() { &e.why } else { &e.text };
        let text = cell(&text.join("<br>"));
        if let Some(reworded) = &opts.issue_ids {
            let mut kind = cell(&kind);
            if reworded.contains(&e.id) {
                kind.push_str(", reworded after filing");
            }
            let issue = diff.was.get(&e.id).map_or("—", String::as_str);
            let _ = writeln!(rows, "| {} | {issue} | {kind} | {text} |", e.id);
        } else {
            let _ = writeln!(rows, "| {} | {} | {text} |", e.id, cell(&kind));
        }
    }
    if rows.is_empty() {
        return rows;
    }
    if opts.issue_ids.is_some() {
        format!("## Requirements\n| ID | Issue | Kind | Requirement |\n|---|---|---|---|\n{rows}\n")
    } else {
        format!("### Requirements\n| ID | Kind | Requirement |\n|---|---|---|\n{rows}\n")
    }
}

fn other_changes(diff: &SpecDiff, opts: &Render) -> String {
    let mut rows = String::new();
    for item in diff.other.iter().filter(|i| opts.cites(i)) {
        let f: Vec<&str> = item.split(" › ").collect();
        let (section, change) = if f.len() > 2 {
            (f[1], f[2])
        } else {
            ("", f[f.len() - 1])
        };
        let _ = writeln!(
            rows,
            "| {} | {} | {} |",
            cell(f[0]),
            cell(section),
            cell(change)
        );
    }
    if rows.is_empty() {
        return rows;
    }
    format!("### Other spec changes\n| File | Section | Change |\n|---|---|---|\n{rows}\n")
}

fn changes(diff: &SpecDiff) -> String {
    let mut out = String::new();
    for rev in &diff.revisions {
        for item in &rev.items {
            let _ = writeln!(out, "- {} — {}", decision(item), rev.who);
        }
    }
    if !out.is_empty() {
        out.push('\n');
    }
    out
}

fn deferred(diff: &SpecDiff) -> String {
    if diff.deferred.is_empty() {
        return String::new();
    }
    format!("### Deferred\n{}\n\n", diff.deferred.join("\n"))
}

/// The text lines joined by spaces, or the why lines joined by `; ` when the
/// entry states no text.
fn spaced(text: &[String], why: &[String]) -> String {
    if text.is_empty() {
        why.join("; ")
    } else {
        text.join(" ")
    }
}

/// A revision item, `<ID>: <what>` written `<ID> — <what>`.
fn decision(item: &str) -> String {
    let b = item.as_bytes();
    let id_len = fixed_id_len(b).or_else(|| {
        let digits = b
            .get(1..)?
            .iter()
            .take_while(|c| c.is_ascii_digit())
            .count();
        (b.first() == Some(&b'S') && digits > 0).then_some(1 + digits)
    });
    match id_len.and_then(|n| Some((n, item[n..].strip_prefix(": ")?))) {
        Some((n, rest)) => format!("{} — {rest}", &item[..n]),
        None => item.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ut_mentions_any_id_needs_a_standalone_token() {
        assert!(mentions_any_id("`site_limit_a` row (R1)"));
        assert!(mentions_any_id("see MB-R-042."));
        assert!(mentions_any_id("R-12 here"));
        assert!(!mentions_any_id("units are amperes throughout"));
        assert!(!mentions_any_id("XR1 and R1x"));
    }

    #[test]
    fn ut_mentions_is_bounded() {
        assert!(mentions("row (R1)", "R1"));
        assert!(!mentions("row (R12)", "R1"));
        assert!(!mentions("row MB-R1", "R1"));
    }

    #[test]
    fn ut_revision_needs_date_and_decider() {
        assert_eq!(revision("- rev 3 — fixed IDs (fe-planner)"), None);
        let r = revision("- rev 4 (2026-10-05) — A; B (user)").unwrap();
        assert_eq!(r.items, ["A", "B"]);
        assert_eq!(r.who, "user");
    }

    #[test]
    fn ut_decision_splits_a_leading_id() {
        assert_eq!(decision("MB-R-1: kept"), "MB-R-1 — kept");
        assert_eq!(decision("S2: dropped"), "S2 — dropped");
        assert_eq!(decision("MB-R-161 kept: x"), "MB-R-161 kept: x");
    }

    #[test]
    fn ut_back_writes_local_ids() {
        let mut diff = SpecDiff::default();
        diff.was.insert("MB-R-161".into(), "R1".into());
        assert_eq!(
            diff.back("MB-R-161 replaces MB-R-002"),
            "R1 replaces MB-R-002"
        );
    }
}

#[cfg(test)]
mod reader_tests {
    use super::*;

    const DIFF: &str = "# Spec diff\n\n## Summary\n- rev 1 (2026-10-05) — first (user)\n\n## MB-R-001\nChange: new\nThe text.\n```\n## MB-R-002\n```\n~~~\n## MB-R-003\n```\n## MB-R-004\n~~~\n";

    #[test]
    fn ut_parse_reads_crlf_like_lf() {
        let crlf = DIFF.replace('\n', "\r\n");
        let lf = SpecDiff::parse(DIFF);
        let cr = SpecDiff::parse(&crlf);
        assert_eq!(lf.revisions.len(), 1);
        assert_eq!(cr.revisions.len(), 1);
        assert_eq!(cr.entries.len(), 1);
        assert_eq!(cr.entries[0].id, "MB-R-001");
        let opts = Render::default();
        assert_eq!(
            render(&cr, Part::Entries, &opts),
            render(&lf, Part::Entries, &opts)
        );
    }

    #[test]
    fn ut_entry_keeps_its_code_block_verbatim() {
        let text = "## MB-R-001\nBefore.\n```sh\n## not a heading\n\n  - x\n```\nAfter.\n";
        let d = SpecDiff::parse(text);
        assert_eq!(d.entries.len(), 1);
        assert_eq!(
            d.entries[0].text,
            [
                "Before.",
                "```sh",
                "## not a heading",
                "",
                "  - x",
                "```",
                "After."
            ]
        );
        assert!(d.entries[0].why.is_empty());
    }

    #[test]
    fn ut_parse_ignores_fenced_headings() {
        let ids: Vec<_> = SpecDiff::parse(DIFF)
            .entries
            .iter()
            .map(|e| e.id.clone())
            .collect();
        assert_eq!(ids, ["MB-R-001"]);
    }
}
