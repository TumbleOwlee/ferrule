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
    /// Findings the architect fixed, in a round event.
    pub fixed: u64,
    /// The check run a round event reports; `None` for a stop event.
    pub round: Option<u32>,
}

impl Logged {
    /// `["scope=<scope>", "findings=<n>", "changed=<n>"]`, or for a round
    /// event `["scope=<scope>", "findings=<n>", "fixed=<n>", "round=<k>"]`.
    #[must_use]
    pub fn args(&self) -> Vec<String> {
        let mut args = vec![
            format!("scope={}", self.check.scope()),
            format!("findings={}", self.findings),
        ];
        match self.round {
            Some(k) => {
                args.push(format!("fixed={}", self.fixed));
                args.push(format!("round={k}"));
            }
            None => args.push(format!("changed={}", self.changed)),
        }
        args
    }
}

/// The distinct finding IDs of `answer` for `check`: words (split on every
/// non-ASCII-alphanumeric char) of `check.letter()` in either case followed by
/// decimal digits without a leading zero, with `1 <= n <= shown`.
#[must_use]
pub fn named(answer: &str, check: Check, shown: usize) -> u64 {
    let ids: BTreeSet<usize> = answer
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter_map(|word| finding_id(word, check).filter(|n| (1..=shown).contains(n)))
        .collect();
    ids.len() as u64
}

/// The number in a finding ID of `check`: its letter in either case followed
/// by decimal digits without a leading zero.
fn finding_id(word: &str, check: Check) -> Option<usize> {
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
    digits.parse::<usize>().ok()
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
                fixed: 0,
                round: None,
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

/// The loop state file in the run directory.
pub const LOOP_FILE: &str = "check-loop.md";

/// The most check runs between a `ready` and the architecture stop.
pub const CAP: u32 = 3;

/// A finding of one check, kept in the loop file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Marked {
    pub check: Check,
    pub text: String,
}

/// The state of the automatic check runs before an architecture stop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Loop {
    /// Check runs started in this loop, 1-based.
    pub run: u32,
    /// The run whose findings were passed back and await the architect's `ready`.
    pub passed_back: Option<u32>,
    /// Per check in `Check::ALL` order, the passed-back run's finding count;
    /// `None` for a check without a result.
    pub results: [Option<usize>; 2],
    /// The passed-back findings, per check in `Findings:` order.
    pub passed: Vec<Marked>,
    /// Findings the architect fixed, per check, in `Check::ALL` order.
    pub fixed: [u64; 2],
    /// Findings marked for the user.
    pub needs_user: Vec<Marked>,
}

fn slot(check: Check) -> usize {
    match check {
        Check::Comprehension => 0,
        Check::ZoomOut => 1,
    }
}

fn count_word<T: ToString>(v: Option<T>) -> String {
    v.map_or_else(|| "-".to_owned(), |n| n.to_string())
}

/// The value after `prefix` on the first line that starts with it.
fn field<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    text.lines()
        .find_map(|l| l.strip_prefix(prefix))
        .map(str::trim)
}

/// The `- ` items under the `head` line, ending at the first other line.
fn list<'a>(text: &'a str, head: &str) -> Vec<&'a str> {
    let mut out = Vec::new();
    let mut rest = text.lines().skip_while(|l| l.trim_end() != head);
    rest.next();
    for line in rest {
        if line.trim().is_empty() {
            continue;
        }
        match line.strip_prefix("- ") {
            Some(f) => out.push(f.trim()),
            None => break,
        }
    }
    out
}

/// The `<scope> <value>` pairs of a comma-separated line value.
fn pairs(value: Option<&str>) -> Vec<(Check, &str)> {
    value
        .into_iter()
        .flat_map(|v| v.split(','))
        .filter_map(|part| {
            let mut words = part.split_whitespace();
            Some((Check::parse(words.next()?)?, words.next()?))
        })
        .collect()
}

fn check_of(letter: &str) -> Option<Check> {
    Check::ALL.into_iter().find(|c| {
        letter.len() == 1 && letter.starts_with(|l: char| l.eq_ignore_ascii_case(&c.letter()))
    })
}

impl Loop {
    /// The loop of a first check run.
    #[must_use]
    pub fn fresh() -> Loop {
        Loop {
            run: 1,
            passed_back: None,
            results: [None, None],
            passed: Vec::new(),
            fixed: [0, 0],
            needs_user: Vec::new(),
        }
    }

    /// The run's loop file; absent or unreadable gives `Loop::fresh()`, and a
    /// line that is missing or unreadable keeps its fresh value.
    #[must_use]
    pub fn load(layout: &Layout, slug: &str) -> Loop {
        let mut lp = Loop::fresh();
        let Ok(text) = std::fs::read_to_string(layout.run_dir(slug).join(LOOP_FILE)) else {
            return lp;
        };
        if let Some(n) = field(&text, "Run:").and_then(|v| v.parse().ok()) {
            lp.run = n;
        }
        if let Some(v) = field(&text, "Passed back:") {
            lp.passed_back = v.parse().ok();
        }
        for (c, n) in pairs(field(&text, "Results:")) {
            lp.results[slot(c)] = n.parse().ok();
        }
        for (c, n) in pairs(field(&text, "Fixed:")) {
            if let Ok(n) = n.parse() {
                lp.fixed[slot(c)] = n;
            }
        }
        for item in list(&text, "Passed:") {
            if let Some((id, body)) = item.split_once(' ')
                && let Some(check) = id.get(..1).and_then(check_of)
                && finding_id(id, check).is_some()
            {
                lp.passed.push(Marked {
                    check,
                    text: body.trim().to_owned(),
                });
            }
        }
        for item in list(&text, "Needs user:") {
            if let Some((id, body)) = item.split_once(' ')
                && let Some(check) = check_of(id)
            {
                lp.needs_user.push(Marked {
                    check,
                    text: body.trim().to_owned(),
                });
            }
        }
        lp
    }

    /// Writes the whole loop file.
    ///
    /// # Errors
    /// Any write failure.
    pub fn save(&self, layout: &Layout, slug: &str) -> std::io::Result<()> {
        let mut out = vec![
            format!("Run: {}", self.run),
            format!("Passed back: {}", count_word(self.passed_back)),
            format!(
                "Results: comprehension {}, zoom-out {}",
                count_word(self.results[0]),
                count_word(self.results[1])
            ),
            "Passed:".to_owned(),
        ];
        for check in Check::ALL {
            for (i, text) in self.passed(check).enumerate() {
                out.push(format!("- {}{} {text}", check.letter(), i + 1));
            }
        }
        out.push(format!(
            "Fixed: comprehension {}, zoom-out {}",
            self.fixed[0], self.fixed[1]
        ));
        out.push("Needs user:".to_owned());
        for m in &self.needs_user {
            out.push(format!("- {} {}", m.check.letter(), m.text));
        }
        out.push(String::new());
        let out = out.join("\n");
        std::fs::write(layout.run_dir(slug).join(LOOP_FILE), out)
    }

    /// Removes the loop file; an absent file is no error.
    ///
    /// # Errors
    /// Any removal failure other than a missing file.
    pub fn remove(layout: &Layout, slug: &str) -> std::io::Result<()> {
        match std::fs::remove_file(layout.run_dir(slug).join(LOOP_FILE)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }

    /// The texts of `check`'s findings marked for the user, in stored order.
    pub fn marked(&self, check: Check) -> impl Iterator<Item = &str> {
        self.needs_user
            .iter()
            .filter(move |m| m.check == check)
            .map(|m| m.text.as_str())
    }

    /// The passed-back findings of `check`, in ID order.
    pub fn passed(&self, check: Check) -> impl Iterator<Item = &str> {
        self.passed
            .iter()
            .filter(move |m| m.check == check)
            .map(|m| m.text.as_str())
    }

    /// Copies the outcomes into `results` and `passed` and sets `passed_back`
    /// to the current run.
    pub fn pass_back(&mut self, outcomes: &[Outcome]) {
        self.results = [None, None];
        self.passed.clear();
        for check in Check::ALL {
            let Some(o) = outcomes.iter().find(|o| o.check == check) else {
                continue;
            };
            self.results[slot(check)] = o.findings.as_ref().map(Vec::len);
            for text in o.findings.iter().flatten() {
                self.passed.push(Marked {
                    check,
                    text: text.clone(),
                });
            }
        }
        self.passed_back = Some(self.run);
    }

    /// Whether the run is the last allowed and an outcome holds a finding.
    #[must_use]
    pub fn cap_reached(&self, outcomes: &[Outcome]) -> bool {
        self.run >= CAP
            && outcomes
                .iter()
                .any(|o| o.findings.as_ref().is_some_and(|f| !f.is_empty()))
    }
}

/// The architect's answer to a pass-back: finding IDs fixed and marked.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Verdict {
    pub fixed: Vec<(Check, usize)>,
    pub needs_user: Vec<(Check, usize)>,
}

fn parse_ids(list: Option<&str>) -> Result<Vec<(Check, usize)>, String> {
    let mut out = Vec::new();
    for item in list.into_iter().flat_map(|l| l.split(',')) {
        let item = item.trim();
        if item == "-" {
            continue;
        }
        let id = Check::ALL
            .into_iter()
            .find_map(|c| finding_id(item, c).map(|n| (c, n)))
            .ok_or_else(|| format!("'{item}' is no finding ID; give them as C1,Z2 or -"))?;
        out.push(id);
    }
    Ok(out)
}

impl Verdict {
    /// Reads the comma-separated ID lists (`C1,z2` or `-`).
    ///
    /// # Errors
    /// An item that is neither `-` nor a finding ID.
    pub fn parse(fixed: Option<&str>, needs_user: Option<&str>) -> Result<Verdict, String> {
        Ok(Verdict {
            fixed: parse_ids(fixed)?,
            needs_user: parse_ids(needs_user)?,
        })
    }

    /// The distinct fixed IDs of `check` within `1..=shown`, minus those also
    /// marked for the user.
    #[must_use]
    pub fn fixed_count(&self, check: Check, shown: usize) -> u64 {
        let ids: BTreeSet<usize> = self
            .fixed
            .iter()
            .filter(|(c, n)| *c == check && (1..=shown).contains(n))
            .filter(|id| !self.needs_user.contains(id))
            .map(|(_, n)| *n)
            .collect();
        ids.len() as u64
    }
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
                    changed: 2,
                    fixed: 0,
                    round: None
                },
                Logged {
                    check: Check::ZoomOut,
                    findings: 1,
                    changed: 1,
                    fixed: 0,
                    round: None
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
            fixed: 0,
            round: None,
        };
        assert_eq!(l.args(), ["scope=zoom-out", "findings=3", "changed=1"]);
    }

    /// GA-R-183 — a round event carries `fixed` and `round` in place of `changed`.
    #[test]
    fn ut_logged_round_args() {
        let l = Logged {
            check: Check::Comprehension,
            findings: 2,
            changed: 0,
            fixed: 1,
            round: Some(2),
        };
        assert_eq!(
            l.args(),
            ["scope=comprehension", "findings=2", "fixed=1", "round=2"]
        );
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

    fn outcome_of(check: Check, findings: Option<&[&str]>) -> Outcome {
        Outcome {
            check,
            path: PathBuf::from("x"),
            findings: findings.map(|f| f.iter().map(|t| (*t).to_owned()).collect()),
        }
    }

    fn full_loop() -> Loop {
        Loop {
            run: 2,
            passed_back: Some(2),
            results: [Some(2), None],
            passed: vec![
                Marked {
                    check: Check::Comprehension,
                    text: "a b".to_owned(),
                },
                Marked {
                    check: Check::Comprehension,
                    text: "c".to_owned(),
                },
                Marked {
                    check: Check::ZoomOut,
                    text: "z".to_owned(),
                },
            ],
            fixed: [1, 4],
            needs_user: vec![
                Marked {
                    check: Check::Comprehension,
                    text: "m one".to_owned(),
                },
                Marked {
                    check: Check::ZoomOut,
                    text: "m two".to_owned(),
                },
            ],
        }
    }

    /// An absent loop file reads as the first run of a fresh loop.
    #[test]
    fn ut_loop_absent_file_is_fresh() {
        let (_d, l) = run(true);
        assert_eq!(Loop::load(&l, "demo"), Loop::fresh());
    }

    /// A saved loop reads back whole, both lists with two items.
    #[test]
    fn ut_loop_round_trips_through_its_file() {
        let (_d, l) = run(true);
        let lp = full_loop();
        lp.save(&l, "demo").unwrap();
        assert_eq!(Loop::load(&l, "demo"), lp);
    }

    /// An unreadable line keeps the fresh value of its field.
    #[test]
    fn ut_loop_unreadable_line_keeps_the_fresh_value() {
        let (_d, l) = run(true);
        let text = "Run: x\nPassed back: 2\nResults: comprehension q, zoom-out 3\nFixed: comprehension 5\n";
        std::fs::write(l.run_dir("demo").join(LOOP_FILE), text).unwrap();
        let lp = Loop::load(&l, "demo");
        assert_eq!(lp.run, 1);
        assert_eq!(lp.passed_back, Some(2));
        assert_eq!(lp.results, [None, Some(3)]);
        assert_eq!(lp.fixed, [5, 0]);
    }

    /// Removing an absent loop file is no error.
    #[test]
    fn ut_loop_remove_of_an_absent_file_is_ok() {
        let (_d, l) = run(true);
        Loop::remove(&l, "demo").unwrap();
        full_loop().save(&l, "demo").unwrap();
        Loop::remove(&l, "demo").unwrap();
        assert!(!l.run_dir("demo").join(LOOP_FILE).exists());
    }

    /// Passing back copies counts and findings by check, not by position.
    #[test]
    fn ut_pass_back_copies_results_and_findings() {
        let mut lp = Loop::fresh();
        lp.pass_back(&[
            outcome_of(Check::ZoomOut, None),
            outcome_of(Check::Comprehension, Some(&["a", "b"])),
        ]);
        assert_eq!(lp.results, [Some(2), None]);
        assert_eq!(lp.passed_back, Some(1));
        assert_eq!(
            lp.passed(Check::Comprehension).collect::<Vec<_>>(),
            ["a", "b"]
        );
        assert_eq!(lp.passed(Check::ZoomOut).count(), 0);
        assert_eq!(lp.marked(Check::ZoomOut).count(), 0);
    }

    /// The cap is reached on the third run with a finding only.
    #[test]
    fn ut_cap_reached_needs_the_third_run_and_a_finding() {
        let with = [outcome_of(Check::Comprehension, Some(&["a"]))];
        let none = [
            outcome_of(Check::Comprehension, Some(&[])),
            outcome_of(Check::ZoomOut, None),
        ];
        let mut lp = Loop::fresh();
        lp.run = 2;
        assert!(!lp.cap_reached(&with));
        lp.run = CAP;
        assert!(lp.cap_reached(&with));
        assert!(!lp.cap_reached(&none));
    }

    /// Verdict lists take IDs in either case and `-`.
    #[test]
    fn ut_verdict_parse_takes_ids_and_dash() {
        let v = Verdict::parse(Some("C1, z2"), Some("-")).unwrap();
        assert_eq!(v.fixed, [(Check::Comprehension, 1), (Check::ZoomOut, 2)]);
        assert!(v.needs_user.is_empty());
        assert_eq!(Verdict::parse(None, None).unwrap(), Verdict::default());
    }

    /// A malformed item is refused with its text.
    #[test]
    fn ut_verdict_parse_refuses_a_malformed_item() {
        for bad in ["C01", "X1", "C"] {
            let e = Verdict::parse(Some(bad), None).unwrap_err();
            assert!(e.contains(&format!("'{bad}' is no finding ID")), "{e}");
        }
    }

    /// GA-E-043 — an ID the passed-back run did not report counts as neither fixed nor marked.
    #[test]
    fn ut_fixed_count_skips_ids_the_run_did_not_report() {
        let v = Verdict::parse(Some("C1,C1,C3,Z1"), None).unwrap();
        assert_eq!(v.fixed_count(Check::Comprehension, 2), 1);
        assert_eq!(v.fixed_count(Check::ZoomOut, 0), 0);
    }

    /// GA-E-044 — an ID named both fixed and `Needs user` counts as marked only.
    #[test]
    fn ut_fixed_count_skips_ids_also_marked() {
        let v = Verdict::parse(Some("C1,C2"), Some("c2")).unwrap();
        assert_eq!(v.fixed_count(Check::Comprehension, 2), 1);
    }
}
