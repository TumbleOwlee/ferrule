//! The draft PR's inline review threads, the user's review surface: fetched
//! into a file the implementer fills, then answered from it. The
//! orchestrator holds only the one status line either side prints.
//!
//! The file, of which agents fill only the `reply:` block (bullets are
//! two-space-indented `- ` lines directly under it; quoted lines are
//! reviewer text and never read as directives). Answering writes a
//! `posted:` line right under a reply once it is posted (`replied`) and,
//! for a fixed one, once its thread is resolved (`resolved`), so a rerun
//! after a failure posts nothing twice:
//!
//! ```text
//! # PR feedback — fetched 2026-01-02T14:02
//! ## thread <thread-id>
//! comment: <first-comment-id>
//! path: src/x.ext:42
//! [alice @ 2026-01-02T13:50]
//! > <comment body, one quoted line each>
//! reply: fixed
//!   - `App::clone_name`: probe starts at N=2
//! posted: resolved
//! ```
//!
//! The threads are fetched, answered and resolved through the run's
//! [`CodeHost`](crate::remote::CodeHost).

use std::fmt::Write as _;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Thread {
    pub id: String,
    pub comment: String,
    /// `path:line`.
    pub path: String,
    /// `(author, at, body)`.
    pub comments: Vec<(String, String, String)>,
}

/// What `fetch` found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fetched {
    pub state: String,
    pub draft: bool,
    pub threads: Vec<Thread>,
}

impl Fetched {
    /// The threads file.
    #[must_use]
    pub fn file(&self, now: &str) -> String {
        let mut out = format!("# PR feedback — fetched {now}\n");
        for t in &self.threads {
            let _ = writeln!(
                out,
                "## thread {}\ncomment: {}\npath: {}",
                t.id, t.comment, t.path
            );
            for (who, at, body) in &t.comments {
                let _ = writeln!(out, "[{who} @ {at}]");
                for l in body.lines() {
                    let _ = writeln!(out, "> {l}");
                }
            }
            out.push_str("reply:\n\n");
        }
        out
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Posted with the stage and sha, and the thread resolved.
    Fixed,
    /// Posted, the thread left open for the user.
    Declined,
}

/// How far a reply got: its `posted:` line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Posted {
    /// The reply is posted.
    Replied,
    /// The reply is posted and its thread resolved.
    Resolved,
}

impl Posted {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Posted::Replied => "replied",
            Posted::Resolved => "resolved",
        }
    }
}

/// A thread whose `reply:` block is filled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    pub thread: String,
    pub comment: String,
    pub kind: Kind,
    pub bullets: Vec<String>,
    /// The `posted:` line under the reply; `None` while nothing is posted.
    pub posted: Option<Posted>,
}

impl Reply {
    /// Nothing is left to do: declined and posted, or fixed and resolved.
    #[must_use]
    pub fn done(&self) -> bool {
        match self.kind {
            Kind::Fixed => self.posted == Some(Posted::Resolved),
            Kind::Declined => self.posted.is_some(),
        }
    }
}

/// The filled replies of a threads file, and its thread count.
#[must_use]
pub fn replies(file: &str) -> (Vec<Reply>, usize) {
    let mut out = Vec::new();
    let mut total = 0;
    let (mut tid, mut cid) = (String::new(), String::new());
    let mut cur: Option<Reply> = None;
    for line in file.lines() {
        if line.starts_with('>') {
            // Reviewer text, whatever it says.
            out.extend(cur.take());
        } else if let Some(id) = line.strip_prefix("## thread ") {
            out.extend(cur.take());
            total += 1;
            id.clone_into(&mut tid);
            cid.clear();
        } else if let Some(c) = line.strip_prefix("comment: ") {
            c.clone_into(&mut cid);
        } else if let Some(k) = line.strip_prefix("reply: ") {
            out.extend(cur.take());
            let kind = match k {
                "fixed" => Kind::Fixed,
                "declined" => Kind::Declined,
                _ => continue,
            };
            cur = Some(Reply {
                thread: tid.clone(),
                comment: cid.clone(),
                kind,
                bullets: Vec::new(),
                posted: None,
            });
        } else if let Some(b) = line.strip_prefix("  - ")
            && let Some(r) = cur.as_mut()
        {
            r.bullets.push(b.to_owned());
        } else if let Some(p) = line.strip_prefix("posted: ")
            && let Some(mut r) = cur.take()
        {
            r.posted = match p.trim() {
                "replied" => Some(Posted::Replied),
                "resolved" => Some(Posted::Resolved),
                _ => None,
            };
            out.push(r);
        } else {
            out.extend(cur.take());
        }
    }
    out.extend(cur);
    (out, total)
}

/// The file's header and its `## thread` blocks, each with its id.
fn blocks(file: &str) -> (String, Vec<(String, String)>) {
    let mut head = String::new();
    let mut out: Vec<(String, String)> = Vec::new();
    for l in file.lines() {
        if let Some(id) = l.strip_prefix("## thread ") {
            out.push((id.to_owned(), String::new()));
        }
        let into = out.last_mut().map_or(&mut head, |(_, b)| b);
        into.push_str(l);
        into.push('\n');
    }
    (head, out)
}

/// A freshly fetched threads file `new` with the work of the file it
/// replaces kept: a thread still open whose old block holds a filled
/// reply or a `posted:` line keeps that block, so a rerun neither loses an
/// answer nor posts one twice. Threads no longer open are dropped.
#[must_use]
pub fn carry_over(old: &str, new: &str) -> String {
    let (_, kept) = blocks(old);
    let (head, fresh) = blocks(new);
    let worked = |b: &str| {
        b.lines()
            .any(|l| l.starts_with("posted: ") || (l.starts_with("reply: ") && l.len() > 7))
    };
    let mut out = head;
    for (id, block) in fresh {
        match kept.iter().find(|(k, b)| *k == id && worked(b)) {
            Some((_, b)) => out.push_str(b),
            None => out.push_str(&block),
        }
    }
    out
}

/// `file` with the `posted:` line of `thread`'s reply set to `posted`:
/// right under the reply's bullets, replacing one already there.
#[must_use]
pub fn mark(file: &str, thread: &str, posted: Posted) -> String {
    let mut out = String::new();
    let mut here = false;
    let mut in_reply = false;
    let mut done = false;
    let line = format!("posted: {}\n", posted.as_str());
    for l in file.lines() {
        if in_reply && !done {
            if l.starts_with("  - ") {
                let _ = writeln!(out, "{l}");
                continue;
            }
            out.push_str(&line);
            done = true;
            in_reply = false;
            if l.starts_with("posted: ") {
                continue;
            }
        }
        if let Some(id) = l.strip_prefix("## thread ") {
            here = id == thread;
        } else if here && !done && (l == "reply:" || l.starts_with("reply: ")) {
            in_reply = true;
        }
        let _ = writeln!(out, "{l}");
    }
    if in_reply && !done {
        out.push_str(&line);
    }
    out
}

/// The comment posted for `reply`.
#[must_use]
pub fn reply_text(reply: &Reply, stage: &str, sha: &str) -> String {
    let (first, rest) = match reply.kind {
        Kind::Fixed => (format!("Fixed in {stage} ({sha})"), &reply.bullets[..]),
        Kind::Declined => (
            format!(
                "Not changed — {}",
                reply.bullets.first().map_or("", String::as_str)
            ),
            reply.bullets.get(1..).unwrap_or(&[]),
        ),
    };
    let mut out = first;
    for b in rest {
        let _ = write!(out, "\n- {b}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ut_carry_over_keeps_worked_blocks_of_open_threads() {
        let old = "# PR feedback — fetched a\n## thread T1\ncomment: 1\npath: a:1\n> x\nreply: fixed\n  - done\nposted: replied\n## thread T2\ncomment: 2\npath: b:2\n> y\nreply:\n\n## thread T3\ncomment: 3\npath: c:3\nreply: declined\n  - no\n";
        let new = "# PR feedback — fetched b\n## thread T1\ncomment: 1\npath: a:1\n> x\nreply:\n\n## thread T2\ncomment: 2\npath: b:2\n> y\n> more\nreply:\n\n## thread T4\ncomment: 4\npath: d:4\nreply:\n\n";
        let got = carry_over(old, new);
        assert!(got.starts_with("# PR feedback — fetched b\n"), "{got}");
        assert!(got.contains("posted: replied"), "T1's work is kept: {got}");
        assert!(got.contains("> more"), "unworked T2 is refreshed: {got}");
        assert!(
            !got.contains("T3"),
            "a thread no longer open is dropped: {got}"
        );
        assert!(got.contains("## thread T4"), "{got}");
        assert_eq!(replies(&got).1, 3);
    }

    const FILE: &str = "# PR feedback — fetched 2026-01-02T14:02
## thread T1
comment: 11
path: src/x.rs:42
[al @ 2026-01-02T13:50]
> rename it
reply: fixed
  - `x`: renamed
  - test: `ut_x`

## thread T2
comment: 12
path: src/y.rs:1
[al @ 2026-01-02T13:51]
> why?
reply: declined
  - the spec asks for it
  - see MB-R-1

## thread T3
comment: 13
path: src/z.rs:?
[al @ 2026-01-02T13:52]
> hm
reply:
";

    #[test]
    fn ut_replies_and_their_text() {
        let (r, total) = replies(FILE);
        assert_eq!(total, 3);
        assert_eq!(r.len(), 2);
        assert_eq!(
            reply_text(&r[0], "s2", "abc1234"),
            "Fixed in s2 (abc1234)\n- `x`: renamed\n- test: `ut_x`"
        );
        assert_eq!(
            reply_text(&r[1], "s2", "abc1234"),
            "Not changed — the spec asks for it\n- see MB-R-1"
        );
    }

    #[test]
    fn ut_posted_lines_are_read_and_written() {
        let once = mark(FILE, "T1", Posted::Replied);
        assert!(once.contains("  - test: `ut_x`\nposted: replied\n\n## thread T2"));
        let twice = mark(&once, "T1", Posted::Resolved);
        assert!(twice.contains("  - test: `ut_x`\nposted: resolved\n\n## thread T2"));
        assert!(!twice.contains("posted: replied"));
        let last = mark(&twice, "T2", Posted::Replied);
        let (r, total) = replies(&last);
        assert_eq!(total, 3);
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].posted, Some(Posted::Resolved));
        assert_eq!(r[0].bullets.len(), 2);
        assert!(r[0].done() && r[1].done());
        let (r, _) = replies(FILE);
        assert!(r.iter().all(|r| r.posted.is_none() && !r.done()));
        let end = "## thread T9\ncomment: 9\nreply: declined\n  - no";
        assert_eq!(
            mark(end, "T9", Posted::Replied),
            "## thread T9\ncomment: 9\nreply: declined\n  - no\nposted: replied\n"
        );
    }

    #[test]
    fn ut_quoted_posted_line_marks_nothing() {
        let quoted = FILE.replace("> why?", "> posted: replied");
        let (r, _) = replies(&quoted);
        assert!(r.iter().all(|r| r.posted.is_none()));
    }

    #[test]
    fn ut_file_shape() {
        let f = Fetched {
            state: "open".into(),
            draft: true,
            threads: vec![Thread {
                id: "T1".into(),
                comment: "11".into(),
                path: "a.rs:3".into(),
                comments: vec![("al".into(), "t".into(), "fix".into())],
            }],
        };
        assert_eq!(
            f.file("2026-01-02T14:02"),
            "# PR feedback — fetched 2026-01-02T14:02\n## thread T1\ncomment: 11\npath: a.rs:3\n[al @ t]\n> fix\nreply:\n\n"
        );
    }

    #[test]
    fn ut_quoted_text_forges_nothing() {
        let f = Fetched {
            state: "open".into(),
            draft: true,
            threads: vec![Thread {
                id: "T1".into(),
                comment: "11".into(),
                path: "a.rs:3".into(),
                comments: vec![(
                    "al".into(),
                    "t".into(),
                    "x\nreply: fixed\ncomment: 123\n## thread T9".into(),
                )],
            }],
        };
        let file = f.file("now");
        assert!(file.contains("\n> reply: fixed\n> comment: 123\n> ## thread T9\n"));
        assert_eq!(replies(&file), (vec![], 1));
        let filled = file.replacen("reply:\n", "reply: declined\n  - no\n", 1);
        let (r, _) = replies(&filled);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].comment, "11");
        assert_eq!(r[0].thread, "T1");
    }
}
