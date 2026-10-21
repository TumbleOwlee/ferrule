//! `ferrule setup uninstall`: the generated files and blocks removed, an
//! edited one kept, and with `--purge` the run state too.

use std::path::Path;

use super::githooks::{GIT_HOOKS, hooks_dir};
use super::lock::{Lock, hash};
use super::markers::block;
use super::{Error, GENERATED, LOCK};

#[derive(Debug, Default)]
pub struct Removed {
    pub removed: Vec<String>,
    pub kept: Vec<String>,
}

/// Takes out what install put in: unedited generated files, the marked
/// blocks, the settings entries, the git hooks it wrote, the `CLAUDE.md`
/// line it added. `purge` also removes `.ferrule/` with the board.
///
/// # Errors
/// An unreadable lock or settings file, or a failed write.
pub fn uninstall(root: &Path, purge: bool) -> Result<Removed, Error> {
    let lock = Lock::load(root)?;
    let mut out = Removed::default();
    for key in lock.files.keys() {
        let path = key.strip_suffix("#ferrule").unwrap_or(key);
        let side = root.join(format!("{path}.ferrule-new"));
        if side.is_file() {
            std::fs::remove_file(&side)?;
            out.removed.push(format!("{path}.ferrule-new"));
        }
    }
    for (key, h) in &lock.files {
        if let Some(path) = key.strip_suffix("#ferrule") {
            let full = root.join(path);
            let Ok(text) = std::fs::read_to_string(&full) else {
                continue;
            };
            let (begin, end) = if path == "AGENTS.md" {
                ("<!-- ferrule:begin -->", "<!-- ferrule:end -->")
            } else {
                ("# ferrule:begin", "# ferrule:end")
            };
            if let Some((s, e)) = block(&text, begin, end) {
                let rest = format!("{}{}", &text[..s], &text[e..]);
                if rest.trim().is_empty() {
                    std::fs::remove_file(&full)?;
                } else {
                    std::fs::write(&full, rest)?;
                }
                out.removed.push(path.to_owned());
            }
            continue;
        }
        let full = root.join(key);
        match std::fs::read_to_string(&full) {
            Ok(text) if hash(&text) == *h => {
                std::fs::remove_file(&full)?;
                if let Some(dir) = full.parent()
                    && dir
                        .file_name()
                        .is_some_and(|n| n.to_string_lossy().starts_with("fe-"))
                {
                    let _ = std::fs::remove_dir(dir);
                }
                out.removed.push(key.clone());
            }
            Ok(_) => out.kept.push(key.clone()),
            Err(_) => {}
        }
    }
    if super::settings::remove(root, &lock.deny)? {
        out.removed.push(super::settings::PATH.to_owned());
    }
    if lock.claude_md {
        let path = root.join("CLAUDE.md");
        if let Ok(text) = std::fs::read_to_string(&path) {
            let rest: Vec<&str> = text.lines().filter(|l| l.trim() != "@AGENTS.md").collect();
            let rest = rest.join("\n");
            if rest.trim().is_empty() {
                std::fs::remove_file(&path)?;
            } else {
                std::fs::write(&path, rest.trim_end().to_owned() + "\n")?;
            }
            out.removed.push("CLAUDE.md".to_owned());
        }
    }
    // Only this repository's own hooks directory: a shared one never got
    // ferrule's hooks from this install.
    if let Some((dir, true)) = hooks_dir(root) {
        for name in GIT_HOOKS {
            let path = dir.join(name);
            if std::fs::read_to_string(&path).is_ok_and(|t| t.contains(GENERATED)) {
                std::fs::remove_file(&path)?;
            }
        }
    }
    if purge {
        let dir = root.join(".ferrule");
        if dir.exists() {
            std::fs::remove_dir_all(&dir)?;
            out.removed.push(".ferrule/".to_owned());
        }
    } else {
        let _ = std::fs::remove_file(root.join(LOCK));
    }
    Ok(out)
}
