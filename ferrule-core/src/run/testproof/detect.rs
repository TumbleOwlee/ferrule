//! Test detection: the tests a unified diff adds, removes or rewrites, read
//! with the declaration pattern of one test command.

use std::collections::BTreeMap;

use regex::Regex;

use super::TestChanges;
use super::syntax::{Grammar, Parsed};

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

/// One match of the declaration pattern in a file side.
struct Declaration {
    name: String,
    /// Bytes of the `name` capture in the joined text.
    span: std::ops::Range<usize>,
    /// Line holding the match's start.
    first: usize,
    /// Line holding the match's end.
    last: usize,
}

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

    /// The matches of the declaration pattern in `text`, in order.
    fn declarations(&self, text: &str) -> Vec<Declaration> {
        let mut starts = vec![0];
        starts.extend(text.match_indices('\n').map(|(i, _)| i + 1));
        let line_of = |byte: usize| starts.partition_point(|s| *s <= byte) - 1;
        self.declare
            .captures_iter(text)
            .filter_map(|c| {
                let whole = c.get(0)?;
                let name = c.name("name")?;
                let first = line_of(whole.start());
                let last = if whole.end() > whole.start() {
                    line_of(whole.end() - 1)
                } else {
                    first
                };
                Some(Declaration {
                    name: name.as_str().to_owned(),
                    span: name.range(),
                    first,
                    last,
                })
            })
            .collect()
    }

    /// Adds the tests found in `lines`, one file's side of a diff, to `side`.
    /// A test starts at the first line of its declaration's match. In a file
    /// of a supported language its body ends at the last line of the syntax
    /// node holding its name, cut before the next declaration when that starts
    /// inside the node. When the parser could not read that node, or in a file
    /// of no supported language, the body ends before the next declaration or
    /// at the end of the lines. A body holds at least its match's lines.
    fn scan(&self, path: &str, lines: &[&str], prefix: &str, side: &mut Side) {
        if lines.is_empty() {
            return;
        }
        let text = lines.join("\n");
        let decls = self.declarations(&text);
        let parsed = if decls.is_empty() {
            None
        } else {
            Grammar::for_path(path).map(|g| Parsed::parse(g, &text))
        };
        for (i, d) in decls.iter().enumerate() {
            let next = decls.get(i + 1).map(|n| n.first);
            let fallback = next.map_or(lines.len() - 1, |n| n.saturating_sub(1));
            let end = match parsed
                .as_ref()
                .map(|p| p.body_end(d.span.clone(), d.first, d.last))
            {
                None | Some(None) => fallback,
                Some(Some(e)) if e > d.last => match next {
                    Some(n) if n <= e => n.saturating_sub(1),
                    _ => e,
                },
                Some(Some(e)) => e,
            };
            let end = end.max(d.last);
            let body = lines[d.first..=end]
                .iter()
                .map(|l| squeeze(l))
                .filter(|l| !l.is_empty())
                .collect();
            side.entry(format!("{prefix}{}", d.name))
                .or_default()
                .push(body);
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
        let mut path = String::new();
        let flush = |path: &str,
                     before: &mut Vec<&str>,
                     after: &mut Vec<&str>,
                     old: &mut Side,
                     new: &mut Side| {
            self.scan(path, before, prefix, old);
            self.scan(path, after, prefix, new);
            before.clear();
            after.clear();
        };
        for line in diff.lines() {
            if line.starts_with("diff --git ") {
                flush(&path, &mut before, &mut after, &mut old, &mut new);
                line.rsplit_once(" b/")
                    .or_else(|| line.rsplit_once(" \"b/"))
                    .map_or("", |(_, p)| p.trim_end_matches('"'))
                    .clone_into(&mut path);
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
        flush(&path, &mut before, &mut after, &mut old, &mut new);
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
        detector_files(files, r"^\s*fn (?P<name>\w+)\(")
    }

    fn detector_files(files: &[&str], declare: &str) -> Detector {
        Detector {
            files: files.iter().map(|f| (*f).to_owned()).collect(),
            declare: regex::RegexBuilder::new(declare)
                .multi_line(true)
                .build()
                .unwrap(),
        }
    }

    fn detector_with(declare: &str) -> Detector {
        detector_files(&["**/*"], declare)
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
            "src/Lib.kt",
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
            "src/Lib.kt",
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

    fn rewritten_only(names_: &[&str]) -> TestChanges {
        TestChanges {
            rewritten: names(names_),
            ..TestChanges::default()
        }
    }

    /// GW-R-111 — the pattern is matched across lines of the whole file.
    #[test]
    fn ut_detect_pattern_spans_lines() {
        let d = diff(
            "src/lib.rs",
            &[
                "+#[test]",
                "+fn t_a() {",
                "+    a();",
                "+}",
                "+fn helper() {",
                "+    b();",
                "+}",
            ],
        );
        let det = detector_with(r"#\[test\]\n\s*fn (?P<name>\w+)\(");
        assert_eq!(det.changes(&d, None).added, names(&["t_a"]));
    }

    /// GW-R-111 — `^` matches at every line's start, so a mid-file removal is seen.
    #[test]
    fn ut_detect_caret_matches_at_every_line() {
        let d = diff(
            "src/lib.rs",
            &[
                " fn t_a() {",
                "     a();",
                " }",
                "",
                "",
                "-fn t_b() {",
                "-    b();",
                "-}",
                "",
                " fn t_c() {",
                "     c();",
                " }",
            ],
        );
        assert_eq!(
            changes(&d),
            TestChanges {
                removed: names(&["t_b"]),
                ..TestChanges::default()
            }
        );
    }

    /// GW-R-112 — a test starts at the first line of its match, decorator included.
    #[test]
    fn ut_detect_test_starts_at_its_match() {
        let d = diff(
            "tests/a.py",
            &[
                "-@pytest.mark.slow",
                "+@pytest.mark.fast",
                " def test_a():",
                "     a()",
            ],
        );
        let det = detector_with(r"@pytest\.mark\.\w+\n\s*def (?P<name>test_\w+)");
        assert_eq!(det.changes(&d, None), rewritten_only(&["test_a"]));
    }

    /// GW-R-114 — a test appended inside a module is only new.
    #[test]
    fn ut_detect_test_appended_in_module_is_only_new() {
        let d = diff(
            "src/lib.rs",
            &[
                " mod tests {",
                "     fn t_a() {",
                "         a();",
                "     }",
                "+    fn t_b() {",
                "+        b();",
                "+    }",
                " }",
            ],
        );
        assert_eq!(
            changes(&d),
            TestChanges {
                added: names(&["t_b"]),
                ..TestChanges::default()
            }
        );
    }

    /// GW-R-114 — a helper between two tests is in no test's body.
    #[test]
    fn ut_detect_helper_between_tests_is_in_no_body() {
        let d = diff(
            "src/lib.rs",
            &[
                " fn t_a() {",
                "     a();",
                " }",
                " fn helper() {",
                "-    one();",
                "+    two();",
                " }",
                " fn t_b() {",
                "     b();",
                " }",
            ],
        );
        let det = detector_with(r"^\s*fn (?P<name>t_\w+)\(");
        assert_eq!(det.changes(&d, None), TestChanges::default());
    }

    /// GW-R-114 — a Python test ends at its last indented line.
    #[test]
    fn ut_detect_python_body_ends_at_dedent() {
        let d = diff(
            "tests/a.py",
            &[
                " @dec",
                " def test_a():",
                "     a()",
                "     b()",
                " ",
                "-x = 1",
                "+x = 2",
            ],
        );
        let det = detector_with(r"^\s*def (?P<name>test_\w+)\(");
        assert_eq!(det.changes(&d, None), TestChanges::default());
    }

    /// GW-R-114 — a one-line test is its match only.
    #[test]
    fn ut_detect_one_line_test_is_its_match() {
        let d = diff(
            "web/a.js",
            &[" test(\"a\", () => x());", "-foo();", "+bar();"],
        );
        let det = detector_with(r#"^\s*test\("(?P<name>[^"]+)""#);
        assert_eq!(det.changes(&d, None), TestChanges::default());
    }

    fn describe_file(tail: &[&str]) -> String {
        let mut lines = vec![
            " describe(\"d\", () => {",
            "   it(\"a\", () => {",
            "     a();",
            "   });",
            "   it(\"b\", () => {",
        ];
        lines.extend_from_slice(tail);
        diff("web/a.js", &lines)
    }

    fn describe_detector() -> Detector {
        detector_with(r#"^\s*(?:describe|it)\("(?P<name>[^"]+)""#)
    }

    /// GW-R-115 — a nested test edit rewrites only that test.
    #[test]
    fn ut_detect_nested_test_edit_rewrites_only_it() {
        let d = describe_file(&["-    b();", "     c();", "   });", " });"]);
        assert_eq!(
            describe_detector().changes(&d, None),
            rewritten_only(&["b"])
        );
    }

    /// GW-E-045 — enclosing lines after a nested test belong to no test's body.
    #[test]
    fn ut_detect_enclosing_lines_after_nested_test_are_in_no_body() {
        let d = describe_file(&["     b();", "   });", "-  tail();", "+  other();", " });"]);
        assert_eq!(
            describe_detector().changes(&d, None),
            TestChanges::default()
        );
    }

    /// GW-R-114 — code appended after a test module rewrites nothing.
    #[test]
    fn ut_detect_code_appended_after_test_module_rewrites_nothing() {
        let d = diff(
            "src/lib.rs",
            &[
                " mod tests {",
                "     fn t_a() {",
                "         a();",
                "     }",
                " }",
                "+fn later() {}",
                "+fn more() {}",
            ],
        );
        let det = detector_with(r"^\s*fn (?P<name>t_\w+)\(");
        assert_eq!(det.changes(&d, None), TestChanges::default());
    }

    /// GW-R-116 — a parse error falls back to the next declaration.
    #[test]
    fn ut_detect_parse_error_falls_back_to_next_declaration() {
        let d = diff(
            "src/lib.rs",
            &[
                " fn t_a() {",
                "     let x = ;",
                " }",
                "  // note",
                "-    one();",
                "+    two();",
                " fn t_b() {}",
            ],
        );
        assert_eq!(changes(&d).rewritten, names(&["t_a"]));
    }

    /// GW-R-117 — a body holds at least its match's lines.
    #[test]
    fn ut_detect_body_holds_its_match() {
        let d = diff(
            "src/Lib.kt",
            &[" // head", "-t_a() t_b() x", "+t_a() t_b() y"],
        );
        let det = detector_with(r"(?P<name>t_\w+)\(\)");
        assert_eq!(det.changes(&d, None), rewritten_only(&["t_a", "t_b"]));
    }

    /// GW-R-109 — a file of no supported language keeps the next-declaration body.
    #[test]
    fn ut_detect_file_of_no_language_keeps_todays_body() {
        let d = diff(
            "src/Lib.kt",
            &[
                " fn t_a() {",
                " }",
                "  // note",
                "-    one();",
                "+    two();",
                " fn t_b() {}",
            ],
        );
        assert_eq!(changes(&d).rewritten, names(&["t_a"]));
    }

    /// GW-R-115, GW-R-117 — two declarations on one row cut the outer body to its own match.
    #[test]
    fn ut_detect_two_declarations_on_one_row_do_not_underflow() {
        let d = diff(
            "web/a.js",
            &[
                " describe(\"a\", () => { it(\"b\", () => {",
                "-  x();",
                "+  y();",
                " });});",
            ],
        );
        let det = detector_with(r#"(?:describe|it)\("(?P<name>[^"]+)""#);
        assert_eq!(det.changes(&d, None), rewritten_only(&["b"]));
    }

    /// GW-R-113 — a path git quotes keeps its extension, so its language is found.
    #[test]
    fn ut_detect_quoted_path_keeps_its_language() {
        let d = diff(
            "src/lib.rs",
            &[
                " mod tests {",
                "     fn t_a() {",
                "         a();",
                "     }",
                "+    fn t_b() {",
                "+        b();",
                "+    }",
                " }",
            ],
        )
        .replace(
            "diff --git a/src/lib.rs b/src/lib.rs",
            "diff --git \"a/src/t\\303\\251.rs\" \"b/src/t\\303\\251.rs\"",
        );
        assert_eq!(
            changes(&d),
            TestChanges {
                added: names(&["t_b"]),
                ..TestChanges::default()
            }
        );
    }
}
