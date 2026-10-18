//! The spec tree: `docs/specs/<area>/requirements.md` and `edge-cases.md`,
//! whose entries carry fixed IDs such as `MB-R-012` or `OC-E-003`.

pub mod diff;

use std::io;
use std::path::{Path, PathBuf};

use crate::markdown;

/// The two entry files of a spec area.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryFile {
    Requirements,
    EdgeCases,
}

impl EntryFile {
    pub const ALL: [EntryFile; 2] = [EntryFile::Requirements, EntryFile::EdgeCases];

    #[must_use]
    pub fn file_name(self) -> &'static str {
        match self {
            EntryFile::Requirements => "requirements.md",
            EntryFile::EdgeCases => "edge-cases.md",
        }
    }
}

/// One requirement or edge case as a spec file states it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecEntry {
    pub id: String,
    /// The `## ` heading the entry sits under, if any.
    pub heading: Option<String>,
    pub text: String,
}

/// Length of the fixed ID at the start of `s`: two capital letters, `-`, `R`
/// or `E`, `-`, digits.
pub(crate) fn fixed_id_len(s: &[u8]) -> Option<usize> {
    let head = s.get(..6)?;
    let shape = head[0].is_ascii_uppercase()
        && head[1].is_ascii_uppercase()
        && head[2] == b'-'
        && matches!(head[3], b'R' | b'E')
        && head[4] == b'-'
        && head[5].is_ascii_digit();
    shape.then(|| 5 + s[5..].iter().take_while(|c| c.is_ascii_digit()).count())
}

fn id_len(s: &str) -> Option<usize> {
    fixed_id_len(s.as_bytes())
}

/// `**<ID>**` at the start of `s`: the ID and what follows the closing `**`.
fn bold_id(s: &str) -> Option<(&str, &str)> {
    let inner = s.strip_prefix("**")?;
    let len = id_len(inner)?;
    let rest = inner[len..].strip_prefix("**")?;
    Some((&inner[..len], rest))
}

/// The entry a line defines, in one of the three forms the specs use:
/// `**ID** — text`, `- **ID** — text`, or the table row `| **ID** | cell | … |`
/// whose cells are joined by ` — `.
fn entry_on_line(line: &str) -> Option<(&str, String)> {
    let listed = line.strip_prefix("- ").unwrap_or(line);
    if let Some((id, rest)) = bold_id(listed)
        && let Some(text) = rest.strip_prefix(" — ")
    {
        return Some((id, text.to_owned()));
    }
    let (id, rest) = bold_id(line.strip_prefix("| ")?)?;
    let cells = rest.strip_prefix(" |")?;
    let trimmed = cells.trim_end_matches(' ');
    let cells = trimmed
        .strip_suffix('|')
        .map_or(cells, |c| c.trim_end_matches(' '));
    let text = cells
        .split('|')
        .map(|c| c.trim_matches([' ', '\t']))
        .filter(|c| !c.is_empty())
        .collect::<Vec<_>>()
        .join(" — ");
    Some((id, text))
}

/// Every entry of one spec file, in file order.
#[must_use]
pub fn entries(text: &str) -> Vec<SpecEntry> {
    let mut heading: Option<String> = None;
    let mut out = Vec::new();
    for line in markdown::lines(text) {
        if let Some(h) = line.strip_prefix("## ") {
            heading = Some(h.to_owned());
            continue;
        }
        if let Some((id, text)) = entry_on_line(line) {
            out.push(SpecEntry {
                id: id.to_owned(),
                heading: heading.clone().filter(|h| !h.is_empty()),
                text,
            });
        }
    }
    out
}

/// The entry files of every area under `specs`, requirements of all areas
/// first, then edge cases; areas in name order.
pub fn entry_files(specs: &Path) -> io::Result<Vec<(String, EntryFile, PathBuf)>> {
    let mut areas: Vec<_> = match std::fs::read_dir(specs) {
        Ok(dir) => dir
            .filter_map(Result::ok)
            .filter(|e| e.path().is_dir())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect(),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(e) => return Err(e),
    };
    areas.sort();
    let mut out = Vec::new();
    for kind in EntryFile::ALL {
        for area in &areas {
            let path = specs.join(area).join(kind.file_name());
            if path.is_file() {
                out.push((area.clone(), kind, path));
            }
        }
    }
    Ok(out)
}

/// Every `*.md` file below `dir`, in path order.
pub fn markdown_files(dir: &Path) -> io::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(next) = pending.pop() {
        for entry in std::fs::read_dir(&next)? {
            let path = entry?.path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|e| e == "md") {
                out.push(path);
            }
        }
    }
    out.sort();
    Ok(out)
}

/// A line of a spec file that mentions `**<id>**`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdHit {
    pub path: PathBuf,
    /// 1-based.
    pub line: usize,
    pub text: String,
}

/// Every line below `dir` that carries `**<id>**`.
pub fn find_id(dir: &Path, id: &str) -> io::Result<Vec<IdHit>> {
    let needle = format!("**{id}**");
    let mut out = Vec::new();
    for path in markdown_files(dir)? {
        let text = std::fs::read_to_string(&path)?;
        for (i, line) in markdown::lines(&text).enumerate() {
            if line.contains(&needle) {
                out.push(IdHit {
                    path: path.clone(),
                    line: i + 1,
                    text: line.to_owned(),
                });
            }
        }
    }
    Ok(out)
}

/// The fixed ID a line defines, `**<ID>**` at its very start.
#[must_use]
pub fn defined_id(line: &str) -> Option<&str> {
    bold_id(line).map(|(id, _)| id)
}

/// The fixed IDs the worktree's branch adds under `docs/specs/` that
/// `reference` (`origin/main`) now defines too: another run landed them
/// first. `None` when the branch shares no history with `reference`.
#[must_use]
pub fn collisions(worktree: &Path, reference: &str) -> Option<Vec<String>> {
    let base = crate::git::out(worktree, &["merge-base", "HEAD", reference])?;
    let diff =
        crate::git::out(worktree, &["diff", &base, "HEAD", "--", "docs/specs"]).unwrap_or_default();
    let added: std::collections::BTreeSet<&str> = diff
        .lines()
        .filter_map(|l| l.strip_prefix('+'))
        .filter_map(defined_id)
        .collect();
    let defined_in = |id: &str, tree: &str| {
        let pattern = format!("^\\*\\*{id}\\*\\*");
        crate::git::ok(
            worktree,
            &["grep", "-q", &pattern, tree, "--", "docs/specs"],
        )
    };
    Some(
        added
            .into_iter()
            .filter(|id| !defined_in(id, &base) && defined_in(id, reference))
            .map(str::to_owned)
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ut_id_shape() {
        assert_eq!(id_len("MB-R-002 x"), Some(8));
        assert_eq!(id_len("OC-E-1"), Some(6));
        assert_eq!(id_len("MB-X-002"), None);
        assert_eq!(id_len("Mb-R-002"), None);
        assert_eq!(id_len("MB-R-"), None);
    }

    #[test]
    fn ut_entries_in_all_three_forms() {
        let text = "# Edge\n\n**MB-E-001** — A slow server times out.\n\n## Client\n| ID | Condition | Behavior |\n|---|---|---|\n| **MB-E-002** | Slow server | the poll is skipped |\n\n- **MB-E-003** — A slow write is retried once.\n**MB-E-004** without a dash\n";
        let got: Vec<_> = entries(text)
            .into_iter()
            .map(|e| (e.id, e.heading, e.text))
            .collect();
        assert_eq!(
            got,
            [
                ("MB-E-001".into(), None, "A slow server times out.".into()),
                (
                    "MB-E-002".into(),
                    Some("Client".into()),
                    "Slow server — the poll is skipped".into()
                ),
                (
                    "MB-E-003".into(),
                    Some("Client".into()),
                    "A slow write is retried once.".into()
                ),
            ]
        );
    }

    #[test]
    fn ut_table_row_without_closing_pipe() {
        assert_eq!(
            entry_on_line("| **MB-R-001** | a | b"),
            Some(("MB-R-001", "a — b".to_owned()))
        );
    }

    #[test]
    fn ut_defined_id_only_at_line_start() {
        assert_eq!(defined_id("**MB-R-002** — x"), Some("MB-R-002"));
        assert_eq!(defined_id("- **MB-R-002** — x"), None);
    }
}
