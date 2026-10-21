//! The lines of an approval file the orchestrator quotes in its prompt,
//! never the rest of the file, and the `Confirm:` block over all files
//! given: slices no slice or two slices claim, and the decisions the user
//! has not seen.

use std::fmt::Write as _;
use std::path::Path;

use crate::artifacts;
use crate::spec::diff::SpecDiff;

/// The items under a line starting `header`, as `- ` lines and (with
/// `nested`) their `  - ` sub-items.
fn block(text: &str, header: &str, nested: bool) -> Vec<String> {
    let mut out = Vec::new();
    for it in artifacts::list(text, header) {
        out.push(format!("- {}", it.text));
        if nested {
            out.extend(it.nested.iter().map(|s| format!("  - {s}")));
        }
    }
    out
}

fn prefixed(text: &str, from: &str, to: &str, out: &mut Vec<String>) {
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix(from) {
            out.push(format!("{to}{rest}"));
        }
    }
}

fn title(text: &str) -> String {
    let first = text.lines().next().unwrap_or("");
    first.strip_prefix("# ").unwrap_or(first).to_owned()
}

fn spec_diff(text: &str, out: &mut Vec<String>) {
    if let Some(goal) = text.lines().find(|l| l.starts_with("Goal: ")) {
        out.push(goal.to_owned());
    }
    let diff = SpecDiff::parse(text);
    let scenarios: Vec<&str> = diff.scenarios.iter().map(|s| s.id.as_str()).collect();
    if !scenarios.is_empty() {
        out.push(format!(
            "Scenarios: {} — {}",
            scenarios.len(),
            scenarios.join(", ")
        ));
    }
    let mut kinds: Vec<(&str, Vec<&str>)> = Vec::new();
    for e in &diff.entries {
        match kinds.iter_mut().find(|(k, _)| *k == e.kind) {
            Some((_, ids)) => ids.push(&e.id),
            None => kinds.push((&e.kind, vec![&e.id])),
        }
    }
    let ids = |k: &str| {
        kinds
            .iter()
            .find(|(kk, _)| *kk == k)
            .map(|(_, v)| v.clone())
    };
    let mut line = String::from("Entries:");
    if let Some(v) = ids("violated") {
        let _ = write!(
            line,
            " no spec change — {} violated: {}",
            v.len(),
            v.join(", ")
        );
    } else {
        let mut sep = "";
        for k in ["new", "changed", "dropped"] {
            if let Some(v) = ids(k) {
                let _ = write!(line, "{sep} {} {k} — {}", v.len(), v.join(", "));
                sep = " ·";
            }
        }
    }
    out.push(line);
    let owners = ["architecture", "plan", "implementation", "unowned"];
    let mut counts = [0usize; 4];
    for l in diff.deferred.iter().filter(|l| l.starts_with("- ")) {
        let owner = owners[..3]
            .iter()
            .position(|o| l.ends_with(&format!(" — {o}")))
            .unwrap_or(3);
        counts[owner] += 1;
    }
    let total: usize = counts.iter().sum();
    if total > 0 {
        let mut line = format!("Deferred: {total}");
        let mut sep = " —";
        for (o, n) in owners.iter().zip(counts) {
            if n > 0 {
                let _ = write!(line, "{sep} {n} {o}");
                sep = " ·";
            }
        }
        out.push(line);
    }
}

fn architecture(text: &str, out: &mut Vec<String>) {
    prefixed(text, "Decide: ", "Architecture: ", out);
    prefixed(text, "Interfaces: ", "Interfaces: ", out);
    let surface = block(text, "Public surface:", false);
    if !surface.is_empty() {
        out.push("Surface:".to_owned());
        out.extend(surface);
    }
}

/// The slice lines, and the confirm lines for spec entries no slice or two
/// slices claim (when the sibling spec-diff.md exists).
fn slices(text: &str, spec_diff: Option<&str>, out: &mut Vec<String>, confirm: &mut Vec<String>) {
    let mut lines: Vec<String> = Vec::new();
    let mut claims: Vec<(String, String)> = Vec::new();
    for s in artifacts::slices(text) {
        let mut cur = format!("- {}: {}", s.n, s.title);
        for (k, v) in &s.fields {
            match k.as_str() {
                "Depends on" => {
                    let _ = write!(cur, " — depends on {v}");
                }
                "Scenarios" => {
                    let _ = write!(cur, " — {v}");
                }
                "Entries" => {
                    let _ = write!(cur, " · {v}");
                }
                _ => continue,
            }
            if k != "Depends on" {
                for id in v
                    .split(',')
                    .map(str::trim)
                    .filter(|w| !w.is_empty() && *w != "none")
                {
                    claims.push((id.to_owned(), s.n.to_string()));
                }
            }
        }
        lines.push(cur);
    }
    if !lines.is_empty() {
        out.push("Slices:".to_owned());
        out.extend(lines);
    }
    let Some(sd) = spec_diff else { return };
    let diff = SpecDiff::parse(sd);
    let wanted = diff.scenarios.iter().map(|s| s.id.as_str()).chain(
        diff.entries
            .iter()
            .filter(|e| e.kind != "dropped" && e.kind != "violated")
            .map(|e| e.id.as_str()),
    );
    for id in wanted {
        let in_slices: Vec<&str> = claims
            .iter()
            .filter(|(c, _)| c == id)
            .map(|(_, s)| s.as_str())
            .collect();
        match in_slices.len() {
            0 => confirm.push(format!("- slices: {id} is in no slice")),
            1 => {}
            _ => confirm.push(format!(
                "- slices: {id} is in slices {}",
                in_slices.join(", ")
            )),
        }
    }
}

fn plan(text: &str, out: &mut Vec<String>) {
    prefixed(text, "Decide: ", "Plan: ", out);
    prefixed(text, "Spec since architecture: ", "Spec since arch: ", out);
    let stops: Vec<String> = artifacts::rows(text)
        .iter()
        .filter_map(|r| Some(format!("- {} — {}", r.sid(), r.stop()?)))
        .collect();
    if !stops.is_empty() {
        out.push("Stops:".to_owned());
        out.extend(stops);
    }
    let v = block(text, "Verification beyond unit tests:", true);
    if !v.is_empty() {
        out.push("Verification:".to_owned());
        out.extend(v);
    }
}

/// The agents' decisions below line `seen`, as
/// `- [<ID> — ]<choice>[ — <why>][ (<agent>)]`; the user's are settled.
fn decisions(text: &str, seen: usize, confirm: &mut Vec<String>) {
    for d in artifacts::decisions(text, seen) {
        if d.by != artifacts::By::Chosen {
            continue;
        }
        let id = match d.tag {
            Some(("spec", Some(t))) => format!("{t} — "),
            _ => String::new(),
        };
        let why = if d.why.is_empty() {
            String::new()
        } else {
            format!(" — {}", d.why)
        };
        let who = d.agent.map_or_else(String::new, |a| format!(" ({a})"));
        confirm.push(format!("- {id}{}{why}{who}", d.choice));
    }
}

fn verdict(text: &str, out: &mut Vec<String>) {
    prefixed(text, "Verdict: ", "Review: ", out);
    prefixed(text, "Not checked — ", "Not checked: ", out);
    prefixed(text, "Spec — ", "Spec: ", out);
    for l in text.lines() {
        if let Some(c) = l.strip_prefix("Spec changes — ")
            && c != "none"
        {
            out.push(format!("Spec changes: {c}"));
        }
    }
    if let Some(header) = text.lines().find(|l| l.starts_with("Minors (folded into ")) {
        let minors = block(text, header, false);
        if !minors.is_empty() {
            out.push(header.to_owned());
            out.extend(minors);
        }
    }
    let needs = block(text, "Needs user", true);
    if !needs.is_empty() {
        out.push("Needs user:".to_owned());
        out.extend(needs);
    }
}

/// The names whose presence prints the `Confirm:` block.
const CONFIRMING: &[&str] = &[
    "spec-diff.md",
    "architecture.summary.md",
    "plan.summary.md",
    "slices.md",
    "decisions.md",
];

/// The quoted lines of `files`, in order, then the `Confirm:` block when
/// one of them confirms decisions. `decisions_seen` is the line count of
/// decisions.md the user last approved. Unreadable files are returned
/// apart.
#[must_use]
pub fn lines(files: &[&Path], decisions_seen: usize) -> (Vec<String>, Vec<String>) {
    let mut out = Vec::new();
    let mut confirm = Vec::new();
    let mut unreadable = Vec::new();
    for path in files {
        let Ok(text) = std::fs::read_to_string(path) else {
            unreadable.push(path.display().to_string());
            continue;
        };
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        match name {
            "spec-diff.md" => spec_diff(&text, &mut out),
            "architecture.summary.md" => architecture(&text, &mut out),
            "slices.md" => {
                let sd = path.parent().map(|d| d.join("spec-diff.md"));
                let sd = sd.and_then(|p| std::fs::read_to_string(p).ok());
                slices(&text, sd.as_deref(), &mut out, &mut confirm);
            }
            "plan.summary.md" => plan(&text, &mut out),
            "decisions.md" => decisions(&text, decisions_seen, &mut confirm),
            "review.verdict.md" => verdict(&text, &mut out),
            "issue.md" => out.push(format!("Issue: {}", title(&text))),
            "split-issue.md" => out.push(format!("Split: {}", title(&text))),
            "pr.md" => out.push(format!("PR: {}", title(&text))),
            "question.md" => out.extend(text.lines().map(str::to_owned)),
            n => {
                if let Some(k) = n
                    .strip_prefix("follow-up-")
                    .and_then(|k| k.strip_suffix(".md"))
                {
                    out.push(format!("Follow-up {k}: {}", title(&text)));
                }
            }
        }
    }
    let confirms = files.iter().any(|p| {
        let s = p.to_string_lossy();
        CONFIRMING.iter().any(|n| s.contains(n))
    });
    if confirms {
        if confirm.is_empty() {
            out.push("Confirm: nothing flagged".to_owned());
        } else {
            out.push("Confirm:".to_owned());
            out.extend(confirm);
        }
    }
    (out, unreadable)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decided(text: &str) -> Vec<String> {
        let mut c = Vec::new();
        decisions(text, 0, &mut c);
        c
    }

    #[test]
    fn ut_decision_forms() {
        assert_eq!(
            decided(
                "- a (chosen) [spec MB-R-1] — why · author\n- b (chosen)\n- c (user) [scope] · author\n- d (chosen) [design] · planner\n"
            ),
            ["- MB-R-1 — a — why (author)", "- b", "- d (planner)"]
        );
    }
}
