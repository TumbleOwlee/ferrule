//! The design checks' result files read for the architecture and epic stops.

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
}
