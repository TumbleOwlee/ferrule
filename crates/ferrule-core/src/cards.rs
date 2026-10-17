//! Task cards as files on the board (`<state>/<slug>.md`,
//! `<state>/<slug>.s<n>.md`), each a frontmatter block and one log line per
//! event. Read-only helpers until the board moves into the state database.

use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};

/// The card files of run `slug` in every state directory under `board`:
/// the parent card and its stage cards.
pub fn run_cards(board: &Path, slug: &str) -> io::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    let stage_prefix = format!("{slug}.s");
    for state in std::fs::read_dir(board)? {
        let state = state?.path();
        if !state.is_dir() {
            continue;
        }
        for card in std::fs::read_dir(&state)? {
            let card = card?.path();
            let Some(name) = card.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let Some(stem) = name.strip_suffix(".md") else {
                continue;
            };
            if stem == slug || stem.starts_with(&stage_prefix) {
                out.push(card);
            }
        }
    }
    out.sort();
    Ok(out)
}

/// The ID of a `spec-delta <ID> changed` log line whose leading key fields
/// (`origin=`, `quote=`, in any order) include an `origin=`: a requirement
/// reworded after the issue was filed.
fn reworded_id(line: &str) -> Option<&str> {
    let mut words = line.splitn(5, ' ');
    let (_, event, id, change) = (words.next()?, words.next()?, words.next()?, words.next()?);
    if event != "spec-delta" || change != "changed" {
        return None;
    }
    let mut rest = words.next().unwrap_or("");
    loop {
        rest = rest.trim_start_matches(' ');
        let (key, value) = if let Some(v) = rest.strip_prefix("origin=") {
            ("origin", v)
        } else if let Some(v) = rest.strip_prefix("quote=") {
            ("quote", v)
        } else {
            return None;
        };
        if key == "origin" {
            return Some(id);
        }
        rest = match value.strip_prefix('"') {
            Some(quoted) => quoted.split_once('"').map_or("", |(_, after)| after),
            None => value.split_once(' ').map_or("", |(_, after)| after),
        };
    }
}

/// IDs of run `slug`'s requirements reworded after filing, from the
/// `spec-delta … changed origin=…` lines on any of its cards.
pub fn reworded_ids(board: &Path, slug: &str) -> io::Result<HashSet<String>> {
    let mut out = HashSet::new();
    for card in run_cards(board, slug)? {
        let text = std::fs::read_to_string(card)?;
        out.extend(text.lines().filter_map(reworded_id).map(str::to_owned));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ut_reworded_needs_origin_among_leading_keys() {
        assert_eq!(
            reworded_id("2026-10-16T20:05 spec-delta MB-R-294 changed origin=user popup"),
            Some("MB-R-294")
        );
        assert_eq!(
            reworded_id(
                "2026-10-16T20:10 spec-delta MB-R-301 changed quote=\"a b\" origin=user box"
            ),
            Some("MB-R-301")
        );
        assert_eq!(
            reworded_id("2026-10-16T20:10 spec-delta MB-R-301 changed popup origin=user"),
            None
        );
        assert_eq!(
            reworded_id("2026-10-16T20:10 spec-delta MB-R-301 added origin=user"),
            None
        );
    }
}
