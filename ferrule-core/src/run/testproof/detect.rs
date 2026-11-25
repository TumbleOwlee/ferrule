//! Test detection: the tests a unified diff adds, removes or rewrites, read
//! with the declaration pattern of one test command.

use std::collections::BTreeMap;

use regex::Regex;

use super::TestChanges;

/// The test files and declaration pattern of one test command.
#[derive(Debug, Clone)]
pub struct Detector {
    pub(super) files: Vec<String>,
    pub(super) declare: Regex,
}

impl PartialEq for Detector {
    fn eq(&self, other: &Self) -> bool {
        self.files == other.files && self.declare.as_str() == other.declare.as_str()
    }
}

impl Eq for Detector {}

/// Test name to the whitespace-free lines of its body, over a whole diff side.
type Side = BTreeMap<String, Vec<Vec<String>>>;

fn squeeze(line: &str) -> String {
    line.split_whitespace().collect()
}

/// Whether `old` holds a line more often than `new` does.
fn loses_a_line(old: &[String], new: &[String]) -> bool {
    let count = |body: &[String], l: &String| body.iter().filter(|x| *x == l).count();
    old.iter().any(|l| count(old, l) > count(new, l))
}

/// Whether a body of `old` is kept whole by no body of `new`.
fn rewritten(old: &[Vec<String>], new: &[Vec<String>]) -> bool {
    old.iter().any(|o| new.iter().all(|n| loses_a_line(o, n)))
}

impl Detector {
    /// `:(glob)<g>` per file glob, for `git diff --` and `ls-files --`.
    #[must_use]
    pub fn pathspecs(&self) -> Vec<String> {
        self.files.iter().map(|g| format!(":(glob){g}")).collect()
    }

    /// Adds the tests found in `lines`, one file's side of a diff, to `side`:
    /// a test's body runs from its declaration to the line before the next
    /// declaration, or to the end of the lines.
    fn scan(&self, lines: &[&str], prefix: &str, side: &mut Side) {
        let mut current: Option<(String, Vec<String>)> = None;
        for line in lines {
            if let Some(name) = self
                .declare
                .captures(line)
                .and_then(|c| c.name("name"))
                .map(|m| m.as_str())
            {
                if let Some((n, body)) = current.take() {
                    side.entry(n).or_default().push(body);
                }
                current = Some((format!("{prefix}{name}"), Vec::new()));
            }
            if let Some((_, body)) = current.as_mut() {
                let sq = squeeze(line);
                if !sq.is_empty() {
                    body.push(sq);
                }
            }
        }
        if let Some((n, body)) = current {
            side.entry(n).or_default().push(body);
        }
    }

    /// The tests a full-context unified diff adds, removes or rewrites, names
    /// prefixed; a test whose body is unchanged in another file is neither.
    #[must_use]
    pub fn changes(&self, diff: &str, prefix: Option<&str>) -> TestChanges {
        let prefix = prefix.unwrap_or("");
        let mut old = Side::new();
        let mut new = Side::new();
        let (mut before, mut after): (Vec<&str>, Vec<&str>) = (Vec::new(), Vec::new());
        let mut header = true;
        let flush =
            |before: &mut Vec<&str>, after: &mut Vec<&str>, old: &mut Side, new: &mut Side| {
                self.scan(before, prefix, old);
                self.scan(after, prefix, new);
                before.clear();
                after.clear();
            };
        for line in diff.lines() {
            if line.starts_with("diff --git ") {
                flush(&mut before, &mut after, &mut old, &mut new);
                header = true;
            } else if line.starts_with("@@") {
                header = false;
            } else if header || line.starts_with('\\') {
            } else if let Some(l) = line.strip_prefix('-') {
                before.push(l);
            } else if let Some(l) = line.strip_prefix('+') {
                after.push(l);
            } else {
                let l = line.strip_prefix(' ').unwrap_or(line);
                before.push(l);
                after.push(l);
            }
        }
        flush(&mut before, &mut after, &mut old, &mut new);
        let mut out = TestChanges::default();
        for name in new.keys().filter(|n| !old.contains_key(*n)) {
            out.added.push(name.clone());
        }
        for (name, body) in &old {
            match new.get(name) {
                None => out.removed.push(name.clone()),
                Some(now) if rewritten(body, now) => out.rewritten.push(name.clone()),
                Some(_) => {}
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn detector(files: &[&str]) -> Detector {
        Detector {
            files: files.iter().map(|f| (*f).to_owned()).collect(),
            declare: Regex::new(r"^\s*fn (?P<name>\w+)\(").unwrap(),
        }
    }

    /// A one-file full-context diff: `lines` carry their own ` `, `-`, `+`.
    fn diff(file: &str, lines: &[&str]) -> String {
        let mut s = format!(
            "diff --git a/{file} b/{file}\nindex 1..2 100644\n--- a/{file}\n+++ b/{file}\n@@ -1,1 +1,1 @@\n"
        );
        for l in lines {
            s.push_str(l);
            s.push('\n');
        }
        s
    }

    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_owned()).collect()
    }

    fn changes(d: &str) -> TestChanges {
        detector(&["**/*.rs"]).changes(d, None)
    }

    /// GW-R-108 — the declared files are the pathspecs a diff is limited to.
    #[test]
    fn ut_detect_finds_tests_only_in_declared_files() {
        assert_eq!(
            detector(&["web/**/*.js", "*.rs"]).pathspecs(),
            [":(glob)web/**/*.js", ":(glob)*.rs"]
        );
        let d = diff("src/lib.rs", &["+fn t_new() {", "+    a();", "+}"]);
        assert_eq!(changes(&d).added, names(&["t_new"]));
    }

    /// GW-R-109 — a body runs from its declaration to the next declaration.
    #[test]
    fn ut_detect_body_runs_to_next_declaration() {
        let d = diff(
            "src/lib.rs",
            &[
                " fn t_a() {",
                "     one();",
                "-    two();",
                " }",
                " fn t_b() {",
                "     three();",
                " }",
            ],
        );
        assert_eq!(
            changes(&d),
            TestChanges {
                rewritten: names(&["t_a"]),
                ..TestChanges::default()
            }
        );
    }

    /// GW-E-038 — a helper between two tests belongs to the earlier test.
    #[test]
    fn ut_detect_helper_between_tests_rewrites_the_earlier() {
        let d = diff(
            "src/lib.rs",
            &[
                " fn t_a() {",
                "     helper();",
                " }",
                " // note",
                "-    one();",
                "+    two();",
                " fn t_b() {}",
            ],
        );
        assert_eq!(changes(&d).rewritten, names(&["t_a"]));
    }

    /// GW-E-018 — a test moved unchanged to another file is neither added nor removed.
    #[test]
    fn ut_detect_moved_test_is_neither() {
        let mut d = diff("src/a.rs", &["-fn t() {", "-    assert!(a);", "-}"]);
        d.push_str(&diff("src/b.rs", &["+fn t() {", "+    assert!(a);", "+}"]));
        assert_eq!(changes(&d), TestChanges::default());
    }

    /// GW-E-023 — a test that only gains lines is not rewritten.
    #[test]
    fn ut_detect_gained_lines_only_is_not_rewritten() {
        let d = diff(
            "src/lib.rs",
            &[" fn t() {", "     assert!(a);", "+    assert!(b);", " }"],
        );
        assert_eq!(changes(&d), TestChanges::default());
    }

    /// GW-R-070 — a lost assertion, even one of two equal lines, is a rewrite.
    #[test]
    fn ut_detect_lost_assertion_is_a_rewrite() {
        let d = diff(
            "src/lib.rs",
            &[" fn t() {", "     assert!(a);", "-    assert!(a);", " }"],
        );
        assert_eq!(changes(&d).rewritten, names(&["t"]));
        let d = diff("src/lib.rs", &["-fn t_gone() {", "-    assert!(a);", "-}"]);
        assert_eq!(changes(&d).removed, names(&["t_gone"]));
    }

    /// GW-R-108, GW-E-022 — names carry the prefix and tests of one name in two files count as one.
    #[test]
    fn ut_detect_names_carry_the_prefix() {
        let d = diff("a.js", &["+fn adds() {", "+}"]);
        assert_eq!(
            detector(&["*.js"]).changes(&d, Some("vitest:")).added,
            names(&["vitest:adds"])
        );
        let mut d = diff("a.rs", &["-fn t() {", "-    x();", "-}"]);
        d.push_str(&diff("b.rs", &[" fn t() {", "     x();", " }"]));
        assert_eq!(changes(&d), TestChanges::default());
    }

    /// GW-E-020 — a declaration the pattern does not match is unseen.
    #[test]
    fn ut_detect_unmatched_declaration_is_unseen() {
        let d = diff("src/lib.rs", &["+test_macro! { t_x }", "+let fn_x = 1;"]);
        assert_eq!(changes(&d), TestChanges::default());
    }
}
