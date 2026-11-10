//! The Rust test scanner: `#[test]` functions in a unified diff and the
//! libtest output of a run.

use std::collections::BTreeMap;

use super::{TestChanges, TestScanner};

/// Scans Rust sources and `cargo test` output.
#[derive(Debug, Clone, Copy, Default)]
pub struct RustScanner;

/// Test name to the whitespace-free lines of its body, over a whole diff side.
type Side = BTreeMap<String, Vec<String>>;

/// Whether `line` is the attribute that marks a test: `#[test]` or a path
/// ending in `::test`, such as `#[tokio::test]`.
fn is_test_attr(line: &str) -> bool {
    line == "#[test]" || (line.starts_with("#[") && line.ends_with("::test]"))
}

/// The name of the function a `fn` line declares, qualifiers skipped.
fn fn_name(line: &str) -> Option<&str> {
    let mut rest = line;
    loop {
        rest = rest.trim_start();
        if let Some(r) = rest.strip_prefix("fn ") {
            let end = r
                .find(|c: char| !(c.is_alphanumeric() || c == '_'))
                .unwrap_or(r.len());
            return (end > 0).then(|| &r[..end]);
        }
        if let Some(r) = rest.strip_prefix("pub(") {
            rest = r.split_once(')')?.1;
            continue;
        }
        let word_end = rest.find(char::is_whitespace)?;
        match &rest[..word_end] {
            "pub" | "async" | "const" | "unsafe" => rest = &rest[word_end..],
            _ => return None,
        }
    }
}

/// The `{` and `}` of `line` outside string and char literals and line comments.
fn braces(line: &str) -> (i64, i64) {
    let (mut open, mut close) = (0, 0);
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' => open += 1,
            '}' => close += 1,
            '"' => {
                while let Some(d) = chars.next() {
                    match d {
                        '\\' => {
                            chars.next();
                        }
                        '"' => break,
                        _ => {}
                    }
                }
            }
            '\'' => {
                let mut ahead = chars.clone();
                let literal = match ahead.next() {
                    Some('\\') => true,
                    Some(_) => ahead.next() == Some('\''),
                    None => false,
                };
                if literal {
                    while let Some(d) = chars.next() {
                        match d {
                            '\\' => {
                                chars.next();
                            }
                            '\'' => break,
                            _ => {}
                        }
                    }
                }
            }
            '/' if chars.peek() == Some(&'/') => break,
            _ => {}
        }
    }
    (open, close)
}

fn squeeze(line: &str) -> String {
    line.split_whitespace().collect()
}

/// Adds the tests found in `lines`, one file's side of a diff, to `side`.
fn scan(lines: &[&str], side: &mut Side) {
    let mut armed = false;
    let mut at = 0;
    while at < lines.len() {
        let t = lines[at].trim();
        if is_test_attr(t) {
            armed = true;
        } else if armed && (t.starts_with("#[") || t.starts_with("///") || t.is_empty()) {
        } else if let Some(name) = armed.then(|| fn_name(t)).flatten() {
            let mut depth = 0_i64;
            let mut opened = false;
            let mut body = Vec::new();
            while at < lines.len() {
                let l = lines[at];
                let (open, close) = braces(l);
                depth += open - close;
                opened |= open > 0;
                let sq = squeeze(l);
                if !sq.is_empty() {
                    body.push(sq);
                }
                if opened && depth <= 0 {
                    break;
                }
                at += 1;
            }
            side.entry(name.to_owned()).or_default().extend(body);
            armed = false;
        } else {
            armed = false;
        }
        at += 1;
    }
}

impl TestScanner for RustScanner {
    fn changes(&self, diff: &str) -> TestChanges {
        let mut old = Side::new();
        let mut new = Side::new();
        let (mut before, mut after): (Vec<&str>, Vec<&str>) = (Vec::new(), Vec::new());
        let mut header = true;
        let mut flush = |before: &mut Vec<&str>, after: &mut Vec<&str>| {
            scan(before, &mut old);
            scan(after, &mut new);
            before.clear();
            after.clear();
        };
        for line in diff.lines() {
            if line.starts_with("diff --git ") {
                flush(&mut before, &mut after);
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
        flush(&mut before, &mut after);
        let mut out = TestChanges::default();
        for name in new.keys().filter(|n| !old.contains_key(*n)) {
            out.added.push(name.clone());
        }
        for (name, body) in &old {
            match new.get(name) {
                None => out.removed.push(name.clone()),
                Some(now) if body.iter().any(|l| !now.contains(l)) => {
                    out.rewritten.push(name.clone());
                }
                Some(_) => {}
            }
        }
        out
    }

    fn failed(&self, output: &str) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for line in output.lines() {
            let name = line
                .strip_prefix("test ")
                .and_then(|r| r.strip_suffix(" ... FAILED"))
                .map(|r| r.strip_suffix(" - should panic").unwrap_or(r));
            if let Some(n) = name
                && !out.iter().any(|o| o == n)
            {
                out.push(n.to_owned());
            }
        }
        out
    }

    fn command(&self, test: &str) -> Vec<String> {
        ["cargo", "test", "--workspace", "--", test]
            .map(str::to_owned)
            .to_vec()
    }

    fn paths(&self) -> &'static [&'static str] {
        &["*.rs"]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        RustScanner.changes(d)
    }

    #[test]
    fn ut_changes_lists_an_added_test() {
        let d = diff(
            "src/lib.rs",
            &[
                " mod tests {",
                "+    #[test]",
                "+    fn t_new() {",
                "+        assert!(true);",
                "+    }",
                " }",
            ],
        );
        assert_eq!(
            changes(&d),
            TestChanges {
                added: names(&["t_new"]),
                ..TestChanges::default()
            }
        );
    }

    #[test]
    fn ut_changes_lists_a_removed_test() {
        let d = diff(
            "src/lib.rs",
            &[
                " mod tests {",
                "-    #[test]",
                "-    fn t_gone() {",
                "-        assert!(true);",
                "-    }",
                " }",
            ],
        );
        assert_eq!(
            changes(&d),
            TestChanges {
                removed: names(&["t_gone"]),
                ..TestChanges::default()
            }
        );
    }

    #[test]
    fn ut_changes_lists_a_test_that_loses_a_line() {
        let d = diff(
            "src/lib.rs",
            &[
                "     #[test]",
                "     fn t() {",
                "         assert!(a);",
                "-        assert!(b);",
                "     }",
            ],
        );
        assert_eq!(
            changes(&d),
            TestChanges {
                rewritten: names(&["t"]),
                ..TestChanges::default()
            }
        );
    }

    /// GW-E-023 — a test that only gains lines is not rewritten.
    #[test]
    fn ut_a_test_that_only_gains_lines_is_not_rewritten() {
        let d = diff(
            "src/lib.rs",
            &[
                "     #[test]",
                "     fn t() {",
                "         assert!(a);",
                "+        assert!(b);",
                "     }",
            ],
        );
        assert_eq!(changes(&d), TestChanges::default());
    }

    /// GW-E-023 — whitespace is ignored when a body is compared.
    #[test]
    fn ut_a_whitespace_change_is_not_a_rewrite() {
        let d = diff(
            "src/lib.rs",
            &[
                "     #[test]",
                "     fn t() {",
                "-        assert!( a );",
                "+            assert!(a);",
                "     }",
            ],
        );
        assert_eq!(changes(&d), TestChanges::default());
    }

    /// GW-E-018 — a test moved unchanged to another file is neither added nor removed.
    #[test]
    fn ut_a_test_moved_unchanged_to_another_file_is_neither() {
        let mut d = diff(
            "src/a.rs",
            &["-#[test]", "-fn t() {", "-    assert!(a);", "-}"],
        );
        d.push_str(&diff(
            "src/b.rs",
            &["+#[test]", "+fn t() {", "+    assert!(a);", "+}"],
        ));
        assert_eq!(changes(&d), TestChanges::default());
    }

    /// GW-E-020 — a doc test and a macro-generated test are not seen.
    #[test]
    fn ut_doc_and_macro_generated_tests_are_not_seen() {
        let d = diff(
            "src/lib.rs",
            &[
                "+/// ```",
                "+/// fn t_doc() { assert!(true); }",
                "+/// ```",
                "+pub fn f() {}",
                "+make_tests! { t_macro }",
                "+macro_rules! make {",
                "+    ($n:ident) => {",
                "+        #[test]",
                "+        fn $n() { assert!(true); }",
                "+    };",
                "+}",
                "+/// #[test]",
                "+/// fn t_doc2() {}",
            ],
        );
        assert_eq!(changes(&d), TestChanges::default());
    }

    /// GW-E-021 — a change to a function outside a test body rewrites no test.
    #[test]
    fn ut_a_helper_change_rewrites_no_test() {
        let d = diff(
            "src/lib.rs",
            &[
                " #[test]",
                " fn t() {",
                "     helper();",
                " }",
                " fn helper() {",
                "-    one();",
                "+    two();",
                " }",
            ],
        );
        assert_eq!(changes(&d), TestChanges::default());
    }

    /// GW-E-022 — two tests of one name in different modules count as one.
    #[test]
    fn ut_two_tests_of_one_name_count_as_one() {
        let d = diff(
            "src/lib.rs",
            &[
                " mod a {",
                "-    #[test]",
                "-    fn t() {",
                "-        assert!(a);",
                "-    }",
                " }",
                " mod b {",
                "     #[test]",
                "     fn t() {",
                "         assert!(a);",
                "     }",
                " }",
            ],
        );
        assert_eq!(changes(&d), TestChanges::default());
    }

    #[test]
    fn ut_an_attribute_path_ending_in_test_is_seen() {
        let d = diff(
            "src/lib.rs",
            &[
                "+#[tokio::test]",
                "+#[ignore]",
                "+/// docs",
                "+",
                "+async fn t_async() {",
                "+    run().await;",
                "+}",
            ],
        );
        assert_eq!(
            changes(&d),
            TestChanges {
                added: names(&["t_async"]),
                ..TestChanges::default()
            }
        );
    }

    #[test]
    fn ut_braces_in_literals_do_not_end_a_body() {
        let d = diff(
            "src/lib.rs",
            &[
                "+#[test]",
                "+fn t_one() {",
                "+    let a = \"{\";",
                "+    let b = '{';",
                "+    let c = '\\'';",
                "+    let d: &'static str = \"\\\"{\";",
                "+}",
                "+#[test]",
                "+fn t_two() {}",
            ],
        );
        assert_eq!(
            changes(&d),
            TestChanges {
                added: names(&["t_one", "t_two"]),
                ..TestChanges::default()
            }
        );
    }

    #[test]
    fn ut_failed_reads_libtest_failures() {
        let out = "running 3 tests\ntest a::ok ... ok\ntest a::bad ... FAILED\ntest b::worse ... FAILED\ntest c::p - should panic ... FAILED\n\nfailures:\n\nfailures:\n    a::bad\n    b::worse\n\ntest result: FAILED. 1 passed; 2 failed\ntest a::bad ... FAILED\n";
        assert_eq!(
            RustScanner.failed(out),
            names(&["a::bad", "b::worse", "c::p"])
        );
    }

    #[test]
    fn ut_command_runs_cargo_test() {
        assert_eq!(
            RustScanner.command("t"),
            names(&["cargo", "test", "--workspace", "--", "t"])
        );
        assert_eq!(RustScanner.paths(), &["*.rs"]);
    }
}
