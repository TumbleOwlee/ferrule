//! What the run commands read from a run's agent-written artifacts:
//! `review.verdict.md`, `plan.summary.md` and the `(rev <n>)` headings.

/// A verdict's `Needs user` list holds an item other than `- none`.
#[must_use]
pub fn needs_user(verdict: &str) -> bool {
    let mut on = false;
    for line in verdict.lines() {
        if line.starts_with("Needs user") {
            on = true;
        } else if on && line.starts_with("- ") {
            if line != "- none" {
                return true;
            }
        } else {
            on = false;
        }
    }
    false
}

/// What a plan summary's `Verification beyond unit tests:` list asks of
/// stage `sid` (`- s1, s3 — demo TUI`), if it names it.
#[must_use]
pub fn verification(summary: &str, sid: &str) -> Option<String> {
    let mut on = false;
    for line in summary.lines() {
        if line.starts_with("Verification beyond unit tests:") {
            on = true;
        } else if on && let Some(item) = line.strip_prefix("- ") {
            let mut parts = item.split(" — ");
            let ids = parts.next().unwrap_or("");
            if ids.split(',').map(str::trim).any(|id| id == sid) {
                return Some(parts.next().unwrap_or("").to_owned());
            }
        } else if !(on && line.starts_with("  - ")) {
            on = false;
        }
    }
    None
}

/// The plan summary's stage rows from s1 on (`| s<n> | …`).
#[must_use]
pub fn stage_rows(summary: &str) -> usize {
    summary
        .lines()
        .filter(|l| {
            l.strip_prefix("| s").is_some_and(|rest| {
                let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
                digits > 0 && !rest.starts_with('0') && rest[digits..].starts_with(' ')
            })
        })
        .count()
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
