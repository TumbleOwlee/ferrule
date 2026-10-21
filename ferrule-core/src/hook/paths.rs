//! The path model the guards share: which files ferrule owns, what a word
//! run from a directory could name (globs and braces included), and where
//! `cd` and `tee` lead.

use std::path::{Component, Path, PathBuf};

/// The most words one word's brace expansion is checked as.
const BRACE_LIMIT: usize = 64;

/// Files no session writes raw, each with the directory it sits in: the
/// board and the lock, which only ferrule writes, and the settings the
/// guard and the deny rules come from, which only the user edits.
pub(super) const OWNED: &[(&str, &str)] = &[
    (".ferrule", "state.db"),
    (".ferrule", "ferrule.lock"),
    (".ferrule", "ferrule.toml"),
    (".claude", "settings.json"),
    (".claude", "settings.local.json"),
];

/// The normal components of `path`, a `..` dropping the one before it.
pub(super) fn components(path: &str) -> Vec<&str> {
    let mut out = Vec::new();
    for c in Path::new(path).components() {
        match c {
            Component::Normal(s) => out.extend(s.to_str()),
            Component::ParentDir => {
                out.pop();
            }
            _ => {}
        }
    }
    out
}

/// Whether shell pattern `pat` (`*`, `?`, `[…]`) matches `name`, ignoring
/// ASCII case as a case-insensitive filesystem does. A leading dot is
/// matched only by a dot, as the shell does.
pub(super) fn glob(pat: &str, name: &str) -> bool {
    let p: Vec<char> = pat.to_ascii_lowercase().chars().collect();
    let n: Vec<char> = name.to_ascii_lowercase().chars().collect();
    if n.first() == Some(&'.') && p.first() != Some(&'.') {
        return false;
    }
    glob_at(&p, &n)
}

fn glob_at(p: &[char], n: &[char]) -> bool {
    match p.split_first() {
        None => n.is_empty(),
        Some(('*', rest)) => (0..=n.len()).any(|i| glob_at(rest, &n[i..])),
        Some(('?', rest)) => !n.is_empty() && glob_at(rest, &n[1..]),
        Some(('[', rest)) => {
            let Some(close) = rest.iter().skip(1).position(|&c| c == ']').map(|i| i + 1) else {
                return n.first() == Some(&'[') && glob_at(rest, &n[1..]);
            };
            let (set, after) = (&rest[..close], &rest[close + 1..]);
            let (negate, set) = match set.split_first() {
                Some(('!' | '^', s)) => (true, s),
                _ => (false, set),
            };
            let Some((&c, n_rest)) = n.split_first() else {
                return false;
            };
            let mut hit = false;
            let mut i = 0;
            while i < set.len() {
                if set.get(i + 1) == Some(&'-') && i + 2 < set.len() {
                    hit |= (set[i]..=set[i + 2]).contains(&c);
                    i += 3;
                } else {
                    hit |= set[i] == c;
                    i += 1;
                }
            }
            hit != negate && glob_at(after, n_rest)
        }
        Some((&c, rest)) => n.first() == Some(&c) && glob_at(rest, &n[1..]),
    }
}

/// The first `{…}` group of `word` (not a `${…}`) holding a top-level
/// `,` or `..`: its byte range, braces included.
pub(super) fn brace_group(word: &str) -> Option<(usize, usize)> {
    let b = word.as_bytes();
    (0..b.len())
        .filter(|&i| b[i] == b'{' && (i == 0 || b[i - 1] != b'$'))
        .find_map(|open| {
            let mut depth = 0;
            let mut list = false;
            for (i, &c) in b.iter().enumerate().skip(open) {
                match c {
                    b'{' => depth += 1,
                    b'}' => {
                        depth -= 1;
                        if depth == 0 {
                            return list.then_some((open, i));
                        }
                    }
                    b',' if depth == 1 => list = true,
                    b'.' if depth == 1 && b.get(i + 1) == Some(&b'.') => list = true,
                    _ => {}
                }
            }
            None
        })
}

/// `word` with its `{a,b}` groups expanded; a `{x..y}` sequence becomes
/// `*`.
pub(super) fn braces(word: &str) -> Vec<String> {
    let Some((open, close)) = brace_group(word) else {
        return vec![word.to_owned()];
    };
    let (pre, inner, post) = (&word[..open], &word[open + 1..close], &word[close + 1..]);
    let mut alts = Vec::new();
    let mut depth = 0;
    let mut from = 0;
    for (i, c) in inner.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => depth -= 1,
            ',' if depth == 0 => {
                alts.push(&inner[from..i]);
                from = i + 1;
            }
            _ => {}
        }
    }
    alts.push(&inner[from..]);
    if alts.len() == 1 {
        alts = vec!["*"];
    }
    alts.iter()
        .flat_map(|a| braces(&format!("{pre}{a}{post}")))
        .take(BRACE_LIMIT)
        .collect()
}

pub(super) fn globbed(word: &str) -> bool {
    word.contains(['*', '?', '[']) || brace_group(word).is_some()
}

/// Whether `f` names file `o` of [`OWNED`] (or a journal file of it), by
/// name or by a glob.
fn owned_file(o: &str, f: &str) -> bool {
    f.to_ascii_lowercase()
        .strip_prefix(o)
        .is_some_and(|r| r.is_empty() || r.starts_with('-'))
        || ["", "-wal", "-shm", "-journal"]
            .iter()
            .any(|s| glob(f, &format!("{o}{s}")))
}

/// `….ferrule/state.db` (and its journal files), any other of [`OWNED`]
/// under its directory, by name, glob or case variant.
pub(super) fn is_owned(path: &str) -> bool {
    let c = components(path);
    matches!(c.as_slice(), [.., d, f] if OWNED.iter().any(|(od, of)| glob(d, od) && owned_file(of, f)))
}

/// `….ferrule` itself, by name, glob or case variant: removing or moving it
/// takes ferrule's files along.
fn is_ferrule_dir(path: &str) -> bool {
    components(path).last().is_some_and(|d| glob(d, ".ferrule"))
}

/// Whether `word`, run from `cwd`, could name one of [`OWNED`] or the
/// `.ferrule` directory: a path to it, a glob or brace form of one, inline
/// code mentioning it, or a glob in the file name directly under
/// `.ferrule/`. A name built from shell variables stays unseen.
pub(super) fn names_owned(cwd: &Path, word: &str) -> bool {
    let in_dir = |d: &str| cwd.file_name().is_some_and(|n| n.eq_ignore_ascii_case(d));
    !word.is_empty()
        && braces(word).iter().any(|w| {
            let lower = w.to_ascii_lowercase();
            let resolved = resolve(cwd, w).to_string_lossy().into_owned();
            is_owned(w)
                || is_owned(&resolved)
                || is_ferrule_dir(w)
                || is_ferrule_dir(&resolved)
                || OWNED.iter().any(|(d, o)| {
                    lower.contains(&format!("{d}/{o}")) || (in_dir(d) && lower.contains(o))
                })
                || [".ferrule", ".claude"].iter().any(|d| {
                    lower
                        .split(&format!("{d}/"))
                        .skip(1)
                        .any(|f| !f.contains('/') && globbed(f))
                        || (in_dir(d) && !w.contains('/') && globbed(w))
                })
        })
}

pub(super) fn is_markdown(path: &str) -> bool {
    Path::new(path)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("md"))
}

/// `…/.ferrule/runs/<slug>/<file>`.
pub(super) fn is_artifact(path: &str, file: &str) -> bool {
    let c = components(path);
    matches!(c.as_slice(), [.., ".ferrule", "runs", _, f] if *f == file)
}

pub(super) fn resolve(cwd: &Path, path: &str) -> PathBuf {
    match path.strip_prefix("~/") {
        Some(rest) => std::env::home_dir().unwrap_or_default().join(rest),
        None => cwd.join(path),
    }
}

/// Words after `tee`'s options, and whether `-a`/`--append` was given.
pub(super) fn tee_targets(args: &[String]) -> (bool, Vec<&str>) {
    let mut append = false;
    let mut targets = Vec::new();
    let mut options = true;
    for a in args {
        match a.as_str() {
            "--" if options => options = false,
            "--append" if options => append = true,
            o if options && o.starts_with('-') && o.len() > 1 => {
                append |= !o.starts_with("--") && o.contains('a');
            }
            t => targets.push(t),
        }
    }
    (append, targets)
}

/// The word `cd <args>` moves to.
pub(super) fn cd_word(args: &[String]) -> Option<&String> {
    args.iter()
        .skip_while(|a| a.starts_with('-') && a.as_str() != "--")
        .find(|a| a.as_str() != "--")
}

/// The directory `cd <args>` moves to, if it exists; a `.ferrule` the
/// guard cannot see (`cd "$(…)/.ferrule"`) is taken as moved to.
pub(super) fn cd_target(cwd: &Path, args: &[String]) -> Option<PathBuf> {
    let target = cd_word(args)?;
    let dir = resolve(cwd, target);
    (dir.is_dir() || is_ferrule_dir(target)).then_some(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ut_owned_paths() {
        assert!(is_owned(".ferrule/state.db"));
        assert!(is_owned("/x/.ferrule/state.db-wal"));
        assert!(is_owned(".ferrule/ferrule.lock"));
        assert!(is_owned(".ferrule/ferrule.toml"));
        assert!(is_owned("/x/.claude/settings.json"));
        assert!(is_owned(".claude/settings.local.json"));
        assert!(!is_owned("/tmp/state.db"));
        assert!(!is_owned("node/settings.json"));
        assert!(!is_owned(".ferrule/runs/x/plan.md"));
    }
}
