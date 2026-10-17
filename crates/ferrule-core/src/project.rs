//! Where the project a command acts on lives.

use std::path::{Path, PathBuf};
use std::process::Command;

/// The project root: `$CLAUDE_PROJECT_DIR` when the agent host sets it, else
/// the current directory as an empty path, so paths joined onto it print
/// relative (`docs/specs/…`, never `./docs/specs/…`).
#[must_use]
pub fn root() -> PathBuf {
    std::env::var_os("CLAUDE_PROJECT_DIR")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_default()
}

/// The project's spec tree, `docs/specs/` under [`root`].
#[must_use]
pub fn specs_dir() -> PathBuf {
    root().join("docs").join("specs")
}

/// The main checkout holding [`root`]: its git common directory's parent,
/// so a command run inside a run's worktree reaches the one board. Outside
/// git, [`root`] itself.
#[must_use]
pub fn main_checkout() -> PathBuf {
    let root = root();
    let dir = if root.as_os_str().is_empty() {
        Path::new(".")
    } else {
        root.as_path()
    };
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["rev-parse", "--path-format=absolute", "--git-common-dir"])
        .output();
    match out {
        Ok(o) if o.status.success() => {
            let common = PathBuf::from(String::from_utf8_lossy(&o.stdout).trim());
            common.parent().map_or(root, Path::to_path_buf)
        }
        _ => root,
    }
}
