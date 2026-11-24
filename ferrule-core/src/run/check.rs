//! The design checks' result files read for the architecture and epic stops.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::artifacts;
use crate::project::Layout;

/// One of the two design checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Check {
    Comprehension,
    ZoomOut,
}

impl Check {
    /// Both checks, in the order the stop lists them.
    pub const ALL: [Check; 2] = [Check::Comprehension, Check::ZoomOut];

    /// The word the stop and the guide use for the check.
    #[must_use]
    pub fn scope(self) -> &'static str {
        match self {
            Check::Comprehension => "comprehension",
            Check::ZoomOut => "zoom-out",
        }
    }

    /// The letter numbering the check's findings.
    #[must_use]
    pub fn letter(self) -> char {
        match self {
            Check::Comprehension => 'C',
            Check::ZoomOut => 'Z',
        }
    }

    /// The result file's name in the run directory.
    #[must_use]
    pub fn file(self) -> &'static str {
        match self {
            Check::Comprehension => "check-comprehension.md",
            Check::ZoomOut => "check-zoom-out.md",
        }
    }

    /// The check a scope word names.
    #[must_use]
    pub fn parse(scope: &str) -> Option<Check> {
        Check::ALL.into_iter().find(|c| c.scope() == scope)
    }
}

/// A check's result file read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub check: Check,
    pub path: PathBuf,
    /// `Some(vec![])`: a current result without findings.
    pub findings: Option<Vec<String>>,
}

/// The current `(rev n)` of a run document, `-` when absent or unstamped.
fn current_rev(dir: &Path, name: &str) -> String {
    std::fs::read_to_string(dir.join(name))
        .ok()
        .and_then(|t| artifacts::rev(&t))
        .map_or_else(|| "-".to_owned(), |n| n.to_string())
}

/// The check's result file read. `findings` is `None` when the file is
/// absent, has no `Findings:` list, or its `Checked:` line names other
/// revisions than the current documents (`spec-diff.md` absent reads as
/// `rev -`).
#[must_use]
pub fn outcome(layout: &Layout, slug: &str, check: Check) -> Outcome {
    let dir = layout.run_dir(slug);
    let path = dir.join(check.file());
    let want = format!(
        "Checked: spec-diff.md rev {}, architecture.md rev {}",
        current_rev(&dir, "spec-diff.md"),
        current_rev(&dir, "architecture.md")
    );
    let findings = std::fs::read_to_string(&path)
        .ok()
        .and_then(|text| findings(&text, &want));
    Outcome {
        check,
        path,
        findings,
    }
}

/// The findings of a result text stamped `want`, `None` without the stamp
/// or the `Findings:` line.
fn findings(text: &str, want: &str) -> Option<Vec<String>> {
    if !text.lines().any(|l| l.trim_end() == want) {
        return None;
    }
    let mut rest = text.lines().skip_while(|l| l.trim_end() != "Findings:");
    rest.next()?;
    let mut out = Vec::new();
    for line in rest {
        if line.trim().is_empty() {
            continue;
        }
        match line.strip_prefix("- ") {
            Some(f) => out.push(f.trim().to_owned()),
            None => break,
        }
    }
    Some(out)
}

/// The stop's `Design checks:` block: per check `- <scope>: <n> finding(s) ·
/// <path>`, `- <scope>: none found · <path>` or `- <scope>: did not run`,
/// then every finding as `- C<n> <text>` or `- Z<n> <text>`; paths relative
/// to `root`.
#[must_use]
pub fn lines(outcomes: &[Outcome], root: &Path) -> Vec<String> {
    let mut out = vec!["Design checks:".to_owned()];
    for o in outcomes {
        let scope = o.check.scope();
        match &o.findings {
            None => out.push(format!("- {scope}: did not run")),
            Some(f) => {
                let path = o.path.strip_prefix(root).unwrap_or(&o.path);
                let path = path.display();
                out.push(match f.len() {
                    0 => format!("- {scope}: none found · {path}"),
                    1 => format!("- {scope}: 1 finding · {path}"),
                    n => format!("- {scope}: {n} findings · {path}"),
                });
            }
        }
    }
    for o in outcomes {
        for (i, f) in o.findings.iter().flatten().enumerate() {
            out.push(format!("- {}{} {f}", o.check.letter(), i + 1));
        }
    }
    out
}

/// The event a check's result is logged as on the run card.
pub const EVENT: &str = "design-check";

/// The answer a stop's design-check lines are logged for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer<'a> {
    Approve,
    Change(&'a str),
    Abandon,
}

/// One `design-check` event to log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Logged {
    pub check: Check,
    pub findings: usize,
    pub changed: u64,
}

impl Logged {
    /// `["scope=<scope>", "findings=<n>", "changed=<n>"]`
    #[must_use]
    pub fn args(&self) -> Vec<String> {
        vec![
            format!("scope={}", self.check.scope()),
            format!("findings={}", self.findings),
            format!("changed={}", self.changed),
        ]
    }
}

/// The distinct finding IDs of `answer` for `check`: words (split on every
/// non-ASCII-alphanumeric char) of `check.letter()` in either case followed by
/// decimal digits without a leading zero, with `1 <= n <= shown`.
#[must_use]
pub fn named(answer: &str, check: Check, shown: usize) -> u64 {
    let ids: BTreeSet<usize> = answer
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter_map(|word| {
            let mut chars = word.chars();
            let first = chars.next()?;
            let digits = chars.as_str();
            if !first.eq_ignore_ascii_case(&check.letter())
                || digits.is_empty()
                || digits.starts_with('0')
                || !digits.bytes().all(|b| b.is_ascii_digit())
            {
                return None;
            }
            digits
                .parse::<usize>()
                .ok()
                .filter(|n| (1..=shown).contains(n))
        })
        .collect();
    ids.len() as u64
}

/// One line per check with a current result; `changed` from `named` for
/// `Answer::Change`, 0 otherwise. Read only: the caller logs in its own
/// transaction.
#[must_use]
pub fn pending(layout: &Layout, slug: &str, answer: Answer<'_>) -> Vec<Logged> {
    Check::ALL
        .into_iter()
        .filter_map(|check| {
            if is_marked(layout, slug, check) {
                return None;
            }
            let found = outcome(layout, slug, check).findings?;
            let changed = match answer {
                Answer::Change(a) => named(a, check, found.len()),
                _ => 0,
            };
            Some(Logged {
                check,
                findings: found.len(),
                changed,
            })
        })
        .collect()
}

/// The last line of a result file whose event a `change` answer logged.
pub const MARK: &str = "Logged: change";

/// Whether the check's result file carries the `MARK` line. An absent or
/// unreadable file is not marked.
#[must_use]
pub fn is_marked(layout: &Layout, slug: &str, check: Check) -> bool {
    std::fs::read_to_string(layout.run_dir(slug).join(check.file()))
        .is_ok_and(|t| t.lines().any(|l| l.trim_end() == MARK))
}

/// Appends the `MARK` line to the check's result file, after a newline when
/// the file lacks one at its end. An absent file is no error.
///
/// # Errors
/// Any read or write failure other than a missing file.
pub fn mark(layout: &Layout, slug: &str, check: Check) -> std::io::Result<()> {
    use std::io::Write;
    let path = layout.run_dir(slug).join(check.file());
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    let mut file = std::fs::OpenOptions::new().append(true).open(&path)?;
    if !text.is_empty() && !text.ends_with('\n') {
        file.write_all(b"\n")?;
    }
    writeln!(file, "{MARK}")
}

/// Removes both result files and returns how many existed; an absent file is
/// no error.
///
/// # Errors
/// Any removal failure other than a missing file.
pub fn clear(layout: &Layout, slug: &str) -> std::io::Result<usize> {
    let mut removed = 0;
    for check in Check::ALL {
        match std::fs::remove_file(layout.run_dir(slug).join(check.file())) {
            Ok(()) => removed += 1,
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
            Err(_) => {}
        }
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(spec_diff: bool) -> (tempfile::TempDir, Layout) {
        let dir = tempfile::tempdir().unwrap();
        let layout = Layout::new(dir.path().to_path_buf());
        let rd = layout.run_dir("demo");
        std::fs::create_dir_all(&rd).unwrap();
        if spec_diff {
            std::fs::write(rd.join("spec-diff.md"), "# demo — spec diff (rev 2)\n").unwrap();
        }
        std::fs::write(
            rd.join("architecture.md"),
            "# demo — architecture (rev 3)\n",
        )
        .unwrap();
        (dir, layout)
    }

    fn write(layout: &Layout, file: &str, stamp: &str, findings: &str) {
        let text = format!("# demo — check\n{stamp}\n## Paraphrase\nx\n{findings}");
        std::fs::write(layout.run_dir("demo").join(file), text).unwrap();
    }

    const CURRENT: &str = "Checked: spec-diff.md rev 2, architecture.md rev 3";

    /// A result stamped with the current revisions is read.
    #[test]
    fn ut_outcome_reads_a_current_result() {
        let (_d, l) = run(true);
        write(
            &l,
            "check-comprehension.md",
            CURRENT,
            "Findings:\n- a\n- b\n",
        );
        let o = outcome(&l, "demo", Check::Comprehension);
        assert_eq!(o.findings, Some(vec!["a".to_owned(), "b".to_owned()]));
    }

    /// GA-R-159 — a current result with an empty `Findings:` list is a result
    /// without findings.
    #[test]
    fn ut_outcome_without_findings_is_empty() {
        let (_d, l) = run(true);
        write(&l, "check-zoom-out.md", CURRENT, "Findings:\n\n");
        assert_eq!(outcome(&l, "demo", Check::ZoomOut).findings, Some(vec![]));
    }

    /// GA-E-029 — a result stamped for other revisions counts as no result.
    #[test]
    fn ut_outcome_of_a_stale_result_is_none() {
        let (_d, l) = run(true);
        for stamp in [
            "Checked: spec-diff.md rev 1, architecture.md rev 3",
            "Checked: spec-diff.md rev 2, architecture.md rev 2",
        ] {
            write(&l, "check-zoom-out.md", stamp, "Findings:\n- a\n");
            assert_eq!(
                outcome(&l, "demo", Check::ZoomOut).findings,
                None,
                "{stamp}"
            );
        }
    }

    /// GA-R-157 — a check without a result file, or a file without `Findings:`,
    /// did not run.
    #[test]
    fn ut_outcome_without_a_file_is_none() {
        let (_d, l) = run(true);
        assert_eq!(outcome(&l, "demo", Check::Comprehension).findings, None);
        write(&l, "check-comprehension.md", CURRENT, "");
        assert_eq!(outcome(&l, "demo", Check::Comprehension).findings, None);
    }

    /// GA-E-029 — without `spec-diff.md` the stamp reads `rev -`.
    #[test]
    fn ut_outcome_without_spec_diff_reads_rev_dash() {
        let (_d, l) = run(false);
        write(
            &l,
            "check-comprehension.md",
            "Checked: spec-diff.md rev -, architecture.md rev 3",
            "Findings:\n- a\n",
        );
        assert_eq!(
            outcome(&l, "demo", Check::Comprehension).findings,
            Some(vec!["a".to_owned()])
        );
    }

    /// GA-R-152, GA-R-153, GA-R-157, GA-R-159 — the block counts per check,
    /// numbers findings per check and names relative paths.
    #[test]
    fn ut_lines_number_findings_per_check() {
        let root = Path::new("/r");
        let o = |check: Check, f: Option<&[&str]>| Outcome {
            check,
            path: root.join(".ferrule/runs/demo").join(check.file()),
            findings: f.map(|f| f.iter().map(|s| (*s).to_owned()).collect()),
        };
        let two = lines(
            &[
                o(Check::Comprehension, Some(&["a", "b"])),
                o(Check::ZoomOut, Some(&[])),
            ],
            root,
        );
        assert_eq!(
            two,
            [
                "Design checks:",
                "- comprehension: 2 findings · .ferrule/runs/demo/check-comprehension.md",
                "- zoom-out: none found · .ferrule/runs/demo/check-zoom-out.md",
                "- C1 a",
                "- C2 b",
            ]
        );
        let one = lines(
            &[
                o(Check::Comprehension, None),
                o(Check::ZoomOut, Some(&["z"])),
            ],
            root,
        );
        assert_eq!(
            one,
            [
                "Design checks:",
                "- comprehension: did not run",
                "- zoom-out: 1 finding · .ferrule/runs/demo/check-zoom-out.md",
                "- Z1 z",
            ]
        );
    }

    /// GA-E-036 — a finding named more than once counts once.
    #[test]
    fn ut_named_counts_distinct_ids() {
        assert_eq!(named("C2 C2 c2", Check::Comprehension, 3), 1);
    }

    /// GA-E-034 — the letter's case does not matter.
    #[test]
    fn ut_named_ignores_case() {
        assert_eq!(named("c2", Check::Comprehension, 3), 1);
    }

    /// GA-E-035 — an ID counts only as a whole word.
    #[test]
    fn ut_named_whole_words_only() {
        assert_eq!(named("C2,", Check::Comprehension, 30), 1);
        assert_eq!(named("(C2)", Check::Comprehension, 30), 1);
        for w in ["C2a", "C02", "C0"] {
            assert_eq!(named(w, Check::Comprehension, 30), 0, "{w}");
        }
        assert_eq!(named("C20", Check::Comprehension, 3), 0);
    }

    /// GA-E-033 — an ID the stop did not show does not count.
    #[test]
    fn ut_named_ignores_ids_not_shown() {
        assert_eq!(named("C4", Check::Comprehension, 3), 0);
    }

    #[test]
    fn ut_named_ignores_the_other_letter() {
        assert_eq!(named("Z1", Check::Comprehension, 3), 0);
    }

    /// GA-R-167 — a check without a result logs nothing.
    #[test]
    fn ut_pending_skips_a_check_without_result() {
        let (_d, l) = run(true);
        write(&l, "check-comprehension.md", CURRENT, "Findings:\n- a\n");
        let got = pending(&l, "demo", Answer::Approve);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].check, Check::Comprehension);
    }

    #[test]
    fn ut_pending_change_counts_named() {
        let (_d, l) = run(true);
        write(
            &l,
            "check-comprehension.md",
            CURRENT,
            "Findings:\n- a\n- b\n",
        );
        write(&l, "check-zoom-out.md", CURRENT, "Findings:\n- z\n");
        let got = pending(&l, "demo", Answer::Change("fix C1, C2 and Z1"));
        assert_eq!(
            got,
            [
                Logged {
                    check: Check::Comprehension,
                    findings: 2,
                    changed: 2
                },
                Logged {
                    check: Check::ZoomOut,
                    findings: 1,
                    changed: 1
                },
            ]
        );
    }

    #[test]
    fn ut_pending_approve_logs_zero() {
        let (_d, l) = run(true);
        write(&l, "check-zoom-out.md", CURRENT, "Findings:\n- z\n");
        let got = pending(&l, "demo", Answer::Approve);
        assert_eq!(got[0].changed, 0);
        assert_eq!(pending(&l, "demo", Answer::Abandon)[0].changed, 0);
    }

    #[test]
    fn ut_clear_removes_both_and_tolerates_absent() {
        let (_d, l) = run(true);
        write(&l, "check-zoom-out.md", CURRENT, "Findings:\n");
        clear(&l, "demo").unwrap();
        for c in Check::ALL {
            assert!(!l.run_dir("demo").join(c.file()).exists());
        }
        clear(&l, "demo").unwrap();
    }

    #[test]
    fn ut_logged_args() {
        let l = Logged {
            check: Check::ZoomOut,
            findings: 3,
            changed: 1,
        };
        assert_eq!(l.args(), ["scope=zoom-out", "findings=3", "changed=1"]);
    }

    #[test]
    fn ut_mark_puts_the_line_on_its_own_line() {
        let (_d, l) = run(true);
        write(&l, "check-zoom-out.md", CURRENT, "Findings:\n- a");
        assert!(!is_marked(&l, "demo", Check::ZoomOut));
        mark(&l, "demo", Check::ZoomOut).unwrap();
        assert!(is_marked(&l, "demo", Check::ZoomOut));
        let text = std::fs::read_to_string(l.run_dir("demo").join("check-zoom-out.md")).unwrap();
        assert!(text.ends_with("- a\nLogged: change\n"), "{text}");
        mark(&l, "demo", Check::Comprehension).unwrap();
    }

    #[test]
    fn ut_marked_result_still_reads_and_is_not_pending() {
        let (_d, l) = run(true);
        write(&l, "check-comprehension.md", CURRENT, "Findings:\n- a\n");
        write(&l, "check-zoom-out.md", CURRENT, "Findings:\n- z\n");
        mark(&l, "demo", Check::Comprehension).unwrap();
        let o = outcome(&l, "demo", Check::Comprehension);
        assert_eq!(o.findings, Some(vec!["a".to_owned()]));
        let got = pending(&l, "demo", Answer::Approve);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].check, Check::ZoomOut);
    }

    #[test]
    fn ut_clear_counts_removed_files() {
        let (_d, l) = run(true);
        write(&l, "check-comprehension.md", CURRENT, "Findings:\n");
        write(&l, "check-zoom-out.md", CURRENT, "Findings:\n");
        assert_eq!(clear(&l, "demo").unwrap(), 2);
        assert_eq!(clear(&l, "demo").unwrap(), 0);
    }
}
