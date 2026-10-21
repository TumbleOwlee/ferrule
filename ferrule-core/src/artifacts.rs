//! What ferrule reads from a run's agent-written artifacts, one reader per
//! shape, so every command reads a file the same way: the labelled lists
//! (`Needs user`, `Public surface:`, `Verification beyond unit tests:`),
//! plan.summary.md's stage rows, decisions.md's lines, the numbered
//! `follow-up-<k>.md` files and the `(rev <n>)` headings. Fenced blocks are
//! never read as structure.

use std::path::{Path, PathBuf};

use crate::markdown;

/// One `- ` item of a labelled list and its `  - ` sub-items, prefixes
/// dropped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item<'a> {
    pub text: &'a str,
    pub nested: Vec<&'a str>,
}

/// The items of every list under a line starting `label`, outside fenced
/// blocks: `- ` lines with their `  - ` sub-items, up to the first other
/// line. `- none` is no item.
#[must_use]
pub fn list<'a>(text: &'a str, label: &str) -> Vec<Item<'a>> {
    let mut out: Vec<Item> = Vec::new();
    let mut on = false;
    let mut last_kept = false;
    for l in markdown::classified(text) {
        let line = l.text;
        if l.fenced || l.delimiter {
            on = false;
        } else if line.starts_with(label) {
            on = true;
            last_kept = false;
        } else if on && let Some(item) = line.strip_prefix("- ") {
            last_kept = item != "none";
            if last_kept {
                out.push(Item {
                    text: item,
                    nested: Vec::new(),
                });
            }
        } else if on && let Some(sub) = line.strip_prefix("  - ") {
            if last_kept && let Some(it) = out.last_mut() {
                it.nested.push(sub);
            }
        } else {
            on = false;
        }
    }
    out
}

/// A verdict's `Needs user` list holds an item other than `- none`.
#[must_use]
pub fn needs_user(verdict: &str) -> bool {
    !list(verdict, "Needs user").is_empty()
}

/// The stage ids of a verification item's head (`s1, s3` or `s1,s3`), when
/// every one is an `s<n>`.
#[must_use]
pub fn verification_ids(ids: &str) -> Option<Vec<&str>> {
    let ids: Vec<&str> = ids.split(',').map(str::trim).collect();
    ids.iter()
        .all(|id| {
            id.strip_prefix('s')
                .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
        })
        .then_some(ids)
}

/// The `Verification beyond unit tests:` items naming stage `sid`
/// (`- s1, s3 — demo TUI`): what each asks, everything after the ids, and
/// its sub-items.
#[must_use]
pub fn verifications<'a>(summary: &'a str, sid: &str) -> Vec<(&'a str, Vec<&'a str>)> {
    list(summary, "Verification beyond unit tests:")
        .into_iter()
        .filter_map(|it| {
            let (ids, what) = it.text.split_once(" — ").unwrap_or((it.text, ""));
            verification_ids(ids)
                .is_some_and(|ids| ids.contains(&sid))
                .then_some((what, it.nested))
        })
        .collect()
}

/// What the first verification item naming `sid` asks, if one does.
#[must_use]
pub fn verification(summary: &str, sid: &str) -> Option<String> {
    verifications(summary, sid)
        .first()
        .map(|(what, _)| (*what).to_owned())
}

/// A row of plan.summary.md's stage table:
/// `| s<n> | What | Pins | Files | Stop |`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row<'a> {
    pub n: u32,
    cells: Vec<&'a str>,
}

impl<'a> Row<'a> {
    /// `s<n>`.
    #[must_use]
    pub fn sid(&self) -> String {
        format!("s{}", self.n)
    }

    /// Cell `i`, trimmed; the stage id is cell 1.
    #[must_use]
    pub fn cell(&self, i: usize) -> &'a str {
        self.cells.get(i).map_or("", |c| c.trim())
    }

    /// The Stop cell when it says `yes…`.
    #[must_use]
    pub fn stop(&self) -> Option<&'a str> {
        Some(self.cell(5)).filter(|c| c.starts_with("yes"))
    }
}

/// The stage rows of a plan summary, s0 included, outside fenced blocks.
#[must_use]
pub fn rows(summary: &str) -> Vec<Row<'_>> {
    markdown::classified(summary)
        .filter(|l| !l.fenced && !l.delimiter && l.text.starts_with("| s"))
        .filter_map(|l| {
            let cells: Vec<&str> = l.text.split('|').collect();
            let id = cells.get(1)?.trim().replace(' ', "");
            let digits = id.strip_prefix('s')?;
            if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            Some(Row {
                n: digits.parse().ok()?,
                cells,
            })
        })
        .collect()
}

/// The plan summary's stage rows from s1 on.
#[must_use]
pub fn stage_rows(summary: &str) -> usize {
    rows(summary).iter().filter(|r| r.n >= 1).count()
}

/// The run's `follow-up-<k>.md` files in `dir`, by `k`.
#[must_use]
pub fn follow_ups(dir: &Path) -> Vec<(u32, PathBuf)> {
    let mut out: Vec<(u32, PathBuf)> = std::fs::read_dir(dir)
        .map(|d| {
            d.filter_map(Result::ok)
                .filter_map(|e| {
                    let name = e.file_name().into_string().ok()?;
                    let k = name.strip_prefix("follow-up-")?.strip_suffix(".md")?;
                    if k.is_empty() || !k.bytes().all(|b| b.is_ascii_digit()) {
                        return None;
                    }
                    Some((k.parse().ok()?, e.path()))
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

/// Who settled a decision: an agent chose it, or the user did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum By {
    Chosen,
    User,
}

/// One decisions.md line:
/// `- <choice> (chosen|user)[ [<kind>[ <ID>]]][ — <why>][ · <agent>]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decision<'a> {
    pub choice: &'a str,
    pub by: By,
    /// The tag's kind (`design`, `process`, `scope`, `spec`) and target id.
    pub tag: Option<(&'a str, Option<&'a str>)>,
    pub why: &'a str,
    pub agent: Option<&'a str>,
}

fn is_id(t: &str) -> bool {
    !t.is_empty()
        && t.bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'-')
}

/// An agent name: lowercase words joined by `-` (`plan-reviewer`).
fn is_agent(a: &str) -> bool {
    !a.is_empty()
        && !a.starts_with('-')
        && !a.ends_with('-')
        && a.bytes().all(|b| b.is_ascii_lowercase() || b == b'-')
}

impl<'a> Decision<'a> {
    /// The line read as a decision; `None` for any other line.
    #[must_use]
    pub fn parse(line: &'a str) -> Option<Self> {
        let item = line.strip_prefix("- ")?;
        let (at, by, word) = [(" (chosen)", By::Chosen), (" (user)", By::User)]
            .into_iter()
            .filter_map(|(w, by)| item.find(w).map(|at| (at, by, w)))
            .min_by_key(|(at, _, _)| *at)?;
        let choice = &item[..at];
        let mut rest = &item[at + word.len()..];
        let mut tag = None;
        if let Some((inner, after)) = rest.strip_prefix(" [").and_then(|r| r.split_once(']')) {
            let mut words = inner.split(' ');
            let kind = words.next().unwrap_or("");
            let target = words.next();
            if !kind.is_empty()
                && kind.bytes().all(|b| b.is_ascii_lowercase())
                && words.next().is_none()
                && target.is_none_or(is_id)
            {
                tag = Some((kind, target));
                rest = after;
            }
        }
        let mut agent = None;
        if let Some((before, a)) = rest.rsplit_once(" · ")
            && is_agent(a)
        {
            agent = Some(a);
            rest = before;
        }
        Some(Decision {
            choice,
            by,
            tag,
            why: rest.strip_prefix(" — ").unwrap_or(rest),
            agent,
        })
    }
}

/// The decisions of decisions.md from line `from` on.
#[must_use]
pub fn decisions(text: &str, from: usize) -> Vec<Decision<'_>> {
    text.lines()
        .skip(from)
        .filter_map(Decision::parse)
        .collect()
}

/// One slice of slices.md: `## Slice <n>: <title>` and its `key: value`
/// lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Slice {
    pub n: u32,
    pub title: String,
    pub fields: Vec<(String, String)>,
}

impl Slice {
    /// The value of `key`, empty when absent.
    #[must_use]
    pub fn get(&self, key: &str) -> &str {
        self.fields
            .iter()
            .find(|(k, _)| k == key)
            .map_or("", |(_, v)| v.as_str())
    }
}

/// The slices of slices.md, outside fenced blocks.
#[must_use]
pub fn slices(text: &str) -> Vec<Slice> {
    let mut out: Vec<Slice> = Vec::new();
    for l in markdown::classified(text).filter(|l| !l.fenced && !l.delimiter) {
        let line = l.text;
        if let Some(rest) = line.strip_prefix("## Slice ")
            && let Some((n, title)) = rest.split_once(": ")
            && !n.is_empty()
            && n.bytes().all(|b| b.is_ascii_digit())
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

/// The numbered questions of question.md: `## <n>. <question>` headings.
#[must_use]
pub fn questions(text: &str) -> usize {
    markdown::classified(text)
        .filter(|l| !l.fenced && !l.delimiter)
        .filter(|l| {
            l.text.strip_prefix("## ").is_some_and(|r| {
                let d = r.bytes().take_while(u8::is_ascii_digit).count();
                d > 0 && r[d..].starts_with(". ")
            })
        })
        .count()
}

/// The lines of decisions.md, the count `decisions-seen=` records and
/// reads back: `str::lines`, so a last line without a newline counts.
#[must_use]
pub fn line_count(text: &str) -> usize {
    text.lines().count()
}

/// The `(rev <n>)` on an artifact's first line.
#[must_use]
pub fn rev(text: &str) -> Option<u32> {
    let first = text.lines().next()?;
    let (_, after) = first.split_once("(rev ")?;
    let (n, _) = after.split_once(')')?;
    n.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ut_needs_user() {
        assert!(needs_user(
            "Verdict: x\n\nNeeds user\n- 1. keep?\n  - a) yes\n"
        ));
        assert!(!needs_user("Needs user\n- none\n"));
        assert!(!needs_user("Verdict: clean\n\nSpec — 2 of 2 pinned\n"));
    }

    #[test]
    fn ut_verification_names_stages() {
        let s = "Decide: x\n\nVerification beyond unit tests:\n- s1, s3 — demo TUI\n  - see it\n- s4 — log\n";
        assert_eq!(verification(s, "s3").as_deref(), Some("demo TUI"));
        assert_eq!(verification(s, "s4").as_deref(), Some("log"));
        assert_eq!(verification(s, "s2"), None);
        let tight = "Verification beyond unit tests:\n- s1,s3 — demo\n";
        assert_eq!(verification(tight, "s3").as_deref(), Some("demo"));
        assert_eq!(verification(tight, "s1").as_deref(), Some("demo"));
    }

    #[test]
    fn ut_list_keeps_sub_items_and_skips_fences() {
        let t = "Needs user\n- 1. keep?\n  - a) yes\n- none\n  - dropped\n\n```\nNeeds user\n- fenced\n```\n";
        assert_eq!(
            list(t, "Needs user"),
            [Item {
                text: "1. keep?",
                nested: vec!["a) yes"]
            }]
        );
    }

    #[test]
    fn ut_verification_keeps_the_whole_ask() {
        let s = "Verification beyond unit tests:\n- s1 — demo — check the log\n";
        assert_eq!(
            verification(s, "s1").as_deref(),
            Some("demo — check the log")
        );
    }

    #[test]
    fn ut_rows_read_the_table_only() {
        let s = "| Stage | What |\n| s0 | a | b | c | — |\n| s1 | b | p | f | yes — new |\n```\n| s9 | x |\n```\n| sx | no |\n";
        let r = rows(s);
        assert_eq!(r.iter().map(|r| r.n).collect::<Vec<_>>(), [0, 1]);
        assert_eq!(r[1].stop(), Some("yes — new"));
        assert_eq!(r[1].cell(2), "b");
        assert_eq!(r[0].stop(), None);
    }

    #[test]
    fn ut_follow_ups_sort_by_number() {
        let dir = tempfile::tempdir().unwrap();
        for k in ["10", "2", "x"] {
            std::fs::write(dir.path().join(format!("follow-up-{k}.md")), "").unwrap();
        }
        let ks: Vec<u32> = follow_ups(dir.path()).into_iter().map(|(k, _)| k).collect();
        assert_eq!(ks, [2, 10]);
    }

    #[test]
    fn ut_decision_lines() {
        let d =
            Decision::parse("- keep retries (chosen) [spec MB-R-1] — fewer calls · plan-reviewer")
                .unwrap();
        assert_eq!(d.choice, "keep retries");
        assert_eq!(d.by, By::Chosen);
        assert_eq!(d.tag, Some(("spec", Some("MB-R-1"))));
        assert_eq!(d.why, "fewer calls");
        assert_eq!(d.agent, Some("plan-reviewer"));
        let u = Decision::parse("- drop it (user) [design]").unwrap();
        assert_eq!(
            (u.by, u.tag, u.why, u.agent),
            (By::User, Some(("design", None)), "", None)
        );
        assert_eq!(Decision::parse("- no marker here"), None);
        assert_eq!(Decision::parse("text"), None);
    }

    #[test]
    fn ut_slices_and_questions_need_a_number() {
        let s = "## Slice : no\n## Slice 2: Core\nDepends on: 1\n## Slice x: no\n";
        let got = slices(s);
        assert_eq!(got.len(), 1);
        assert_eq!((got[0].n, got[0].get("Depends on")), (2, "1"));
        assert_eq!(questions("# Q\n## 1. a?\n## . b?\n## 2. c?\n"), 2);
    }

    #[test]
    fn ut_line_count_counts_an_unterminated_last_line() {
        assert_eq!(line_count("- a\n- b"), 2);
        assert_eq!(line_count("- a\n- b\n"), 2);
    }

    #[test]
    fn ut_stage_rows_skip_s0() {
        assert_eq!(
            stage_rows("| s0 | a |\n| s1 | b |\n| s12 | c |\n| Stage |\n"),
            2
        );
    }

    #[test]
    fn ut_rev() {
        assert_eq!(rev("# ns — plan summary (rev 3)\n"), Some(3));
        assert_eq!(rev("# ns\n"), None);
    }
}
