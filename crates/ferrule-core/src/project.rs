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

/// The checkout holding [`root`] — a worktree's own top level, not the
/// main checkout's — for commands that write committed files. Outside git,
/// [`root`] itself.
#[must_use]
pub fn toplevel() -> PathBuf {
    let root = root();
    let dir = if root.as_os_str().is_empty() {
        Path::new(".")
    } else {
        root.as_path()
    };
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["rev-parse", "--show-toplevel"])
        .output();
    match out {
        Ok(o) if o.status.success() => PathBuf::from(String::from_utf8_lossy(&o.stdout).trim()),
        _ => root,
    }
}

/// Where a run's files live under the main checkout.
#[derive(Debug, Clone)]
pub struct Layout {
    pub root: PathBuf,
}

impl Layout {
    #[must_use]
    pub fn new(root: PathBuf) -> Self {
        Layout { root }
    }

    /// `.ferrule/runs/<slug>/`, the run's agent-written artifacts.
    #[must_use]
    pub fn run_dir(&self, slug: &str) -> PathBuf {
        self.root.join(crate::store::DIR).join("runs").join(slug)
    }

    /// `.ferrule/worktrees/<slug>/`, the run's git worktree.
    #[must_use]
    pub fn worktree(&self, slug: &str) -> PathBuf {
        self.root
            .join(crate::store::DIR)
            .join("worktrees")
            .join(slug)
    }

    /// A path as a card states it: absolute, or under the main checkout.
    #[must_use]
    pub fn resolve(&self, path: &str) -> PathBuf {
        self.root.join(path)
    }

    /// One of the run's artifacts, `None` when absent.
    #[must_use]
    pub fn artifact(&self, slug: &str, name: &str) -> Option<String> {
        std::fs::read_to_string(self.run_dir(slug).join(name)).ok()
    }
}
