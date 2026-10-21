//! `spec next`: the next free fixed IDs of an area's series, counted over
//! the specs on a ref and, for a run, its worktree, spec-diff.md and the
//! `spec-delta … added` lines, so two runs or a retired ID never collide.

use std::path::Path;

use crate::card::typed::{Change, Name, Typed};
use crate::project::Layout;
use crate::refusal::Refusal;
use crate::spec;
use crate::store::Board;

/// Why no IDs were handed out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdsRefused {
    /// The prefix is not two capital letters.
    Prefix,
    /// The ref names no commit.
    Ref,
    /// `git grep` failed on the ref.
    Grep,
    /// No card for the run.
    Card,
    /// The numbers would pass `u32::MAX`.
    Range,
}

impl IdsRefused {
    /// The `next=refused` line.
    #[must_use]
    pub fn refusal(&self) -> Refusal {
        match self {
            IdsRefused::Prefix => {
                Refusal::new("next", "prefix").hint("a prefix is two capital letters, e.g. MB")
            }
            IdsRefused::Ref => Refusal::new("next", "ref").hint("fetch REF or pass --ref"),
            IdsRefused::Grep => {
                Refusal::new("next", "grep").hint("git grep failed on REF; report it to the user")
            }
            IdsRefused::Card => Refusal::card("next"),
            IdsRefused::Range => Refusal::new("next", "range").hint(format!(
                "lower --count; the numbers would pass {}",
                u32::MAX
            )),
        }
    }
}

/// The IDs handed out and the highest one taken before them (`-` when
/// none is).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ids {
    pub next: Vec<String>,
    pub last: String,
}

/// What to count: the area's `prefix`, the edge series or requirements,
/// how many to hand out, the ref and optionally a run.
pub struct Ask<'a> {
    pub prefix: &'a str,
    pub edge: bool,
    pub count: u32,
    pub reference: &'a str,
    pub run: Option<&'a str>,
}

/// The next `ask.count` IDs.
///
/// # Errors
/// A file could not be read or the board failed.
pub fn next(
    db: Option<&Board>,
    layout: &Layout,
    root: &Path,
    ask: &Ask<'_>,
) -> std::io::Result<Result<Ids, IdsRefused>> {
    let prefix = ask.prefix;
    if prefix.len() != 2 || !prefix.bytes().all(|b| b.is_ascii_uppercase()) {
        return Ok(Err(IdsRefused::Prefix));
    }
    let series = if ask.edge { 'E' } else { 'R' };
    if crate::git::commit(root, ask.reference).is_none() {
        return Ok(Err(IdsRefused::Ref));
    }
    // whole lines: `highest` checks the word boundaries
    let needle = format!("{prefix}-{series}-");
    let Some(on_ref) = crate::git::grep(
        root,
        &["-hF", "-e", &needle, ask.reference, "--", "docs/specs"],
    ) else {
        return Ok(Err(IdsRefused::Grep));
    };
    let mut texts = vec![on_ref];
    if let Some(slug) = ask.run {
        match db.map(|db| run_texts(db, layout, slug)).transpose()? {
            Some(Some(found)) => texts.extend(found),
            _ => return Ok(Err(IdsRefused::Card)),
        }
    }
    let best = texts
        .iter()
        .filter_map(|t| spec::highest(prefix, series, t))
        .max_by_key(|(n, _)| *n);
    let width = best.map_or(3, |(_, w)| w.max(3));
    let id = |n: u32| format!("{prefix}-{series}-{n:0width$}");
    let first = best.map_or(Some(1), |(n, _)| n.checked_add(1));
    let Some((first, end)) = first.and_then(|f| Some((f, f.checked_add(ask.count)?))) else {
        return Ok(Err(IdsRefused::Range));
    };
    Ok(Ok(Ids {
        next: (first..end).map(id).collect(),
        last: best.map_or_else(|| "-".to_owned(), |(n, _)| id(n)),
    }))
}

/// The texts run `slug` holds IDs in; `None` without its card.
fn run_texts(db: &Board, layout: &Layout, slug: &str) -> std::io::Result<Option<Vec<String>>> {
    let read = db.read(|r| {
        if r.state(slug)?.is_none() {
            return Ok(None);
        }
        Ok::<_, rusqlite::Error>(Some((
            r.worktree(layout, slug, slug)?,
            r.run_events(slug, &[Name::SpecDelta])?,
        )))
    });
    let Some((wt, deltas)) = read.map_err(std::io::Error::other)? else {
        return Ok(None);
    };
    let mut texts = Vec::new();
    let specs = wt.join("docs/specs");
    if specs.is_dir() {
        for path in spec::markdown_files(&specs)? {
            texts.push(std::fs::read_to_string(path)?);
        }
    }
    if let Ok(diff) = std::fs::read_to_string(layout.run_dir(slug).join("spec-diff.md")) {
        texts.push(diff);
    }
    for e in deltas {
        if let Typed::SpecDelta {
            id,
            change: Some(Change::Added),
            ..
        } = e.typed()
        {
            texts.push(id.to_owned());
        }
    }
    Ok(Some(texts))
}
