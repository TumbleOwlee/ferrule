//! The begin/end markers of the blocks ferrule keeps inside a project's
//! own files, and the checks that they pair up before anything is written.

use std::path::Path;

use super::Error;
use super::items::Item;

/// The byte spans of the lines that are exactly `marker`, `\r\n` or `\n`
/// endings included.
fn marker_lines(text: &str, marker: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut at = 0;
    for line in text.split_inclusive('\n') {
        if line.trim_end_matches(['\n', '\r']) == marker {
            out.push((at, at + line.len()));
        }
        at += line.len();
    }
    out
}

/// What is wrong with the file's markers, if anything: a block ferrule
/// cannot place without guessing which lines are its own.
pub(super) fn marker_problem(text: &str, begin: &str, end: &str) -> Option<String> {
    let (b, e) = (marker_lines(text, begin), marker_lines(text, end));
    match (b.as_slice(), e.as_slice()) {
        ([], []) => None,
        ([(s, _)], [(t, _)]) if s < t => None,
        ([_], [_]) => Some(format!("'{end}' comes before '{begin}'")),
        ([], _) => Some(format!("'{end}' without '{begin}'")),
        (_, []) => Some(format!("'{begin}' without '{end}'")),
        _ => Some(format!("'{begin}' or '{end}' appears more than once")),
    }
}

/// The byte span of the marked block, end line included: one begin line
/// and the first end line after it.
pub(super) fn block(text: &str, begin: &str, end: &str) -> Option<(usize, usize)> {
    let (s, _) = *marker_lines(text, begin).first()?;
    let (_, e) = *marker_lines(text, end).iter().find(|(t, _)| *t > s)?;
    Some((s, e))
}

/// Refuses a block item whose file has unpaired or repeated markers.
pub(super) fn check_markers(root: &Path, items: &[Item]) -> Result<(), Error> {
    for item in items {
        if let Item::Block { begin, end, .. } = item
            && let Ok(text) = std::fs::read_to_string(root.join(item.path()))
            && let Some(why) = marker_problem(&text, begin, end)
        {
            return Err(Error::Markers(item.path().to_owned(), why));
        }
    }
    Ok(())
}
