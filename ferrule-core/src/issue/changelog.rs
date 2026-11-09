//! A milestone description's changelog: the edits a kept spec change makes
//! to its text and the entry recorded under `## Changelog`.

use std::fmt::Write as _;

/// The section every entry is appended to.
pub const HEADING: &str = "## Changelog";

/// One replacement in the description.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    pub old: String,
    pub new: String,
}

/// A parsed `milestone-change.md`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub decision: String,
    pub summary: String,
    pub edits: Vec<Edit>,
}

/// One changelog line.
#[derive(Debug, Clone, Copy)]
pub struct Entry<'a> {
    pub at: &'a str,
    pub author: &'a str,
    pub decision: &'a str,
    pub summary: &'a str,
    /// Hidden in the entry, so a rerun finds it.
    pub sum: &'a str,
}

/// An old text the description does not hold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stale(pub String);

#[must_use]
pub fn parse(text: &str) -> Option<Change> {
    let mut lines = text.lines();
    let decision = lines.next()?.strip_prefix("Decision: ")?.trim().to_owned();
    let summary = lines.next()?.strip_prefix("Summary: ")?.trim().to_owned();
    let mut edits = Vec::new();
    let mut old: Vec<&str> = Vec::new();
    let mut new: Vec<&str> = Vec::new();
    let mut at = Block::Outside;
    for line in lines {
        match (line, at) {
            ("@@ old", Block::Outside) => at = Block::Old,
            ("@@ new", Block::Old) => at = Block::New,
            ("@@ end", Block::New) => {
                let o = old.join("\n");
                if o.is_empty() {
                    return None;
                }
                edits.push(Edit {
                    old: o,
                    new: new.join("\n"),
                });
                old.clear();
                new.clear();
                at = Block::Outside;
            }
            (l, Block::Old) => old.push(l),
            (l, Block::New) => new.push(l),
            _ => {}
        }
    }
    (at == Block::Outside).then_some(Change {
        decision,
        summary,
        edits,
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Block {
    Outside,
    Old,
    New,
}

/// The text above the changelog heading and the changelog from it on
/// (empty without one).
fn split(description: &str) -> (&str, &str) {
    let mut at = 0;
    for line in description.split_inclusive('\n') {
        if line.trim_end() == HEADING {
            return description.split_at(at);
        }
        at += line.len();
    }
    (description, "")
}

#[must_use]
pub fn has_entry(description: &str, sum: &str) -> bool {
    description.contains(&format!("ferrule:change sum={sum} -->"))
}

/// The edits applied and the entry appended; `Stale` names an old text not found.
///
/// # Errors
/// `Stale` when an edit's old text is absent above the changelog.
pub fn apply(description: &str, change: &Change, entry: &Entry<'_>) -> Result<String, Stale> {
    let (above, log) = split(description);
    if let Some(e) = change.edits.iter().find(|e| !above.contains(&e.old)) {
        return Err(Stale(e.old.clone()));
    }
    let mut text = above.to_owned();
    for e in &change.edits {
        text = text.replacen(&e.old, &e.new, 1);
    }
    text.push_str(log);
    Ok(append(&text, entry))
}

/// `description` with `entry` appended under the changelog heading.
#[must_use]
pub fn append(description: &str, entry: &Entry<'_>) -> String {
    let (above, log) = split(description);
    let body = log.strip_prefix(HEADING).unwrap_or("").trim_matches('\n');
    let mut out = format!("{}\n\n{HEADING}\n\n", above.trim_end_matches('\n'));
    if !body.is_empty() {
        out.push_str(body);
        out.push('\n');
    }
    let Entry {
        at,
        author,
        decision,
        summary,
        sum,
    } = entry;
    let _ = writeln!(
        out,
        "- {at} UTC · @{author} · {decision} — {summary} <!-- ferrule:change sum={sum} -->"
    );
    out
}

/// `rendered` followed by the changelog of `existing`.
#[must_use]
pub fn keep_changelog(rendered: &str, existing: &str) -> String {
    let (_, log) = split(existing);
    let log = if log.is_empty() {
        HEADING
    } else {
        log.trim_end()
    };
    format!("{}\n\n{log}\n", rendered.trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry<'a>() -> Entry<'a> {
        Entry {
            at: "2026-11-09T10:00",
            author: "ann",
            decision: "keep retries",
            summary: "retries stay",
            sum: "abc123",
        }
    }

    const CHANGE: &str = "Decision: keep retries\nSummary: retries stay\n\n@@ old\none\ntwo\n@@ new\nthree\n@@ end\n\n@@ old\nx\n@@ new\n@@ end\n";

    /// A change file yields its decision, summary and edits.
    #[test]
    fn ut_parse_reads_the_change() {
        let c = parse(CHANGE).unwrap();
        assert_eq!(c.decision, "keep retries");
        assert_eq!(c.summary, "retries stay");
        assert_eq!(
            c.edits,
            [
                Edit {
                    old: "one\ntwo".into(),
                    new: "three".into()
                },
                Edit {
                    old: "x".into(),
                    new: String::new()
                }
            ]
        );
        assert_eq!(parse("Decision: d\nSummary: s\n").unwrap().edits, []);
        assert_eq!(parse("Summary: s\nDecision: d\n"), None);
        assert_eq!(
            parse("Decision: d\nSummary: s\n\n@@ old\n@@ new\ny\n@@ end\n"),
            None
        );
    }

    /// A block with no `@@ end` is no change.
    #[test]
    fn ut_parse_refuses_an_unterminated_block() {
        assert_eq!(
            parse("Decision: d\nSummary: s\n\n@@ old\na\n@@ new\nb\n"),
            None
        );
        assert_eq!(parse("Decision: d\nSummary: s\n\n@@ old\na\n"), None);
    }

    /// RU-R-021 — the changelog is the last section, with or without a heading before.
    #[test]
    fn ut_append_keeps_the_section_last() {
        let bare = append("Goal\n\ntext\n", &entry());
        assert!(bare.starts_with("Goal\n\ntext\n\n## Changelog\n\n- "));
        assert!(bare.ends_with("-->\n"));
        let again = append(
            &bare,
            &Entry {
                sum: "def",
                ..entry()
            },
        );
        assert_eq!(again.matches(HEADING).count(), 1);
        assert!(again.find("sum=abc123").unwrap() < again.find("sum=def").unwrap());
        assert!(again.ends_with("sum=def -->\n"));
    }

    /// RU-R-021 — an entry carries time, author, decision, summary and its hidden sum.
    #[test]
    fn ut_append_records_time_author_decision_and_summary() {
        assert!(append("x\n", &entry()).contains(
            "- 2026-11-09T10:00 UTC · @ann · keep retries — retries stay <!-- ferrule:change sum=abc123 -->\n"
        ));
    }

    /// A recorded sum is found, another is not.
    #[test]
    fn ut_has_entry_finds_a_recorded_sum() {
        let d = append("x\n", &entry());
        assert!(has_entry(&d, "abc123"));
        assert!(!has_entry(&d, "abc12"));
        assert!(!has_entry("x\n", "abc123"));
    }

    /// One missing old text changes nothing and is named.
    #[test]
    fn ut_apply_checks_every_old_text_first() {
        let c = parse(CHANGE).unwrap();
        let ok = apply("Goal\none\ntwo\nx\nx\n", &c, &entry()).unwrap();
        assert!(ok.starts_with("Goal\nthree\n\nx\n"));
        assert!(ok.contains("## Changelog"));
        let stale = apply("Goal\none\ntwo\n", &c, &entry());
        assert_eq!(stale, Err(Stale("x".into())));
        let in_log = append(
            "Goal\none\ntwo\n",
            &Entry {
                decision: "x",
                ..entry()
            },
        );
        assert_eq!(apply(&in_log, &c, &entry()), Err(Stale("x".into())));
    }

    /// A rendered text keeps the existing history.
    #[test]
    fn ut_keep_changelog_preserves_history() {
        let existing = append("old\n", &entry());
        let out = keep_changelog("new\n\n", &existing);
        assert!(out.starts_with("new\n\n## Changelog\n\n- "));
        assert!(!out.contains("old"));
        assert_eq!(keep_changelog("new\n", "old\n"), "new\n\n## Changelog\n");
    }
}
