//! The early stops a run makes, from the size of its brief or, without one,
//! of its architecture.

use crate::project::Layout;
use crate::spec::diff::SpecDiff;

/// The early stops a run makes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EarlyStops {
    /// No architecture step: the brief is approved at gate 2's stop.
    Gate2Only,
    /// Brief and architecture in one stop; gate 2 a notice.
    Combined,
    /// Brief, architecture and plan stops one by one.
    Separate,
}

/// The `new` and `changed` entries of a brief.
#[must_use]
pub fn brief_size(spec_diff: &str) -> usize {
    SpecDiff::parse(spec_diff)
        .entries
        .iter()
        .filter(|e| e.kind == "new" || e.kind == "changed")
        .count()
}

/// The `new` and `changed` rows of the `## Components` table.
#[must_use]
pub fn arch_size(architecture: &str) -> usize {
    let mut in_components = false;
    let mut change_col = None;
    let mut count = 0;
    for line in architecture.lines() {
        if let Some(h) = line.strip_prefix("## ") {
            in_components = h.trim() == "Components";
            change_col = None;
            continue;
        }
        if !in_components || !line.trim_start().starts_with('|') {
            continue;
        }
        let cells: Vec<&str> = line
            .trim()
            .trim_matches('|')
            .split('|')
            .map(str::trim)
            .collect();
        match change_col {
            None => change_col = cells.iter().position(|c| c.eq_ignore_ascii_case("Change")),
            Some(i) => {
                if cells.get(i).is_some_and(|c| *c == "new" || *c == "changed") {
                    count += 1;
                }
            }
        }
    }
    count
}

/// One entry or none: `Gate2Only`; up to three: `Combined`; more:
/// `Separate`. Without a brief the architecture's components decide: up to
/// three `Combined`, more `Separate`; without either `Gate2Only`.
#[must_use]
pub fn early_stops(layout: &Layout, slug: &str) -> EarlyStops {
    if let Some(brief) = layout.artifact(slug, "spec-diff.md") {
        match brief_size(&brief) {
            0 | 1 => EarlyStops::Gate2Only,
            2 | 3 => EarlyStops::Combined,
            _ => EarlyStops::Separate,
        }
    } else if let Some(arch) = layout.artifact(slug, "architecture.md") {
        if arch_size(&arch) <= 3 {
            EarlyStops::Combined
        } else {
            EarlyStops::Separate
        }
    } else {
        EarlyStops::Gate2Only
    }
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;

    use super::*;

    fn brief(entries: &[&str]) -> String {
        let mut s = String::from("# ns — spec diff (rev 1)\n\n");
        for (i, k) in entries.iter().enumerate() {
            let _ = write!(s, "## R{i} — {k} · requirements\nText.\n\n");
        }
        s
    }

    fn arch(rows: usize) -> String {
        let mut s = String::from(
            "## Components\n| Component | Kind | Change | Role |\n|---|---|---|---|\n",
        );
        for i in 0..rows {
            let _ = writeln!(s, "| `c{i}` | module | new | r |");
        }
        s.push_str("| `old` | module | unchanged | r |\n");
        s
    }

    /// GA-R-116, GA-R-118 — only `new` and `changed` entries count.
    #[test]
    fn ut_brief_size_counts_new_and_changed() {
        assert_eq!(
            brief_size(&brief(&["new", "changed", "violated", "dropped"])),
            2
        );
        assert_eq!(brief_size(&brief(&["violated", "violated"])), 0);
    }

    /// GA-R-126, GA-R-127 — only `new` and `changed` component rows count.
    #[test]
    fn ut_arch_size_counts_new_and_changed() {
        assert_eq!(arch_size(&arch(3)), 3);
        let mixed = "## Components\n| Component | Change |\n|---|---|\n| a | new |\n| b | changed |\n| c | unchanged |\n\n## Other\n| x | new |\n";
        assert_eq!(arch_size(mixed), 2);
    }

    /// GA-R-116, GA-R-117, GA-R-118, GA-R-126, GA-R-127, GA-R-128 — the early stops by size.
    #[test]
    fn ut_early_stops_by_size() {
        let dir = tempfile::tempdir().unwrap();
        let layout = Layout {
            root: dir.path().to_owned(),
        };
        let run = dir.path().join(".ferrule/runs/ns");
        std::fs::create_dir_all(&run).unwrap();
        assert_eq!(early_stops(&layout, "ns"), EarlyStops::Gate2Only);
        for (n, want) in [
            (0, EarlyStops::Gate2Only),
            (1, EarlyStops::Gate2Only),
            (2, EarlyStops::Combined),
            (3, EarlyStops::Combined),
            (4, EarlyStops::Separate),
        ] {
            std::fs::write(run.join("spec-diff.md"), brief(&vec!["new"; n])).unwrap();
            assert_eq!(early_stops(&layout, "ns"), want, "{n} entries");
        }
        std::fs::remove_file(run.join("spec-diff.md")).unwrap();
        for (n, want) in [(3, EarlyStops::Combined), (4, EarlyStops::Separate)] {
            std::fs::write(run.join("architecture.md"), arch(n)).unwrap();
            assert_eq!(early_stops(&layout, "ns"), want, "{n} components");
        }
    }
}
