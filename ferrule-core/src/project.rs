//! Where the project a command acts on lives. Each is resolved once per
//! process: a command runs in one place.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

/// `git rev-parse <args>` run in `dir`, its trimmed output.
fn rev_parse(dir: &Path, args: &[&str]) -> Option<PathBuf> {
    let dir = if dir.as_os_str().is_empty() {
        Path::new(".")
    } else {
        dir
    };
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .arg("rev-parse")
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    Some(PathBuf::from(String::from_utf8_lossy(&out.stdout).trim()))
}

/// The project root: `$CLAUDE_PROJECT_DIR` when the agent host sets it,
/// else the top of the checkout holding the current directory. At that
/// top (or outside git) it is an empty path, so paths joined onto it print
/// relative (`docs/specs/…`, never `./docs/specs/…`); from a subdirectory
/// it is the top's absolute path, so a command finds the same files there.
#[must_use]
pub fn root() -> PathBuf {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        if let Some(dir) = std::env::var_os("CLAUDE_PROJECT_DIR").filter(|v| !v.is_empty()) {
            return PathBuf::from(dir);
        }
        let here = std::env::current_dir()
            .ok()
            .and_then(|d| d.canonicalize().ok());
        match rev_parse(Path::new(""), &["--show-toplevel"]) {
            Some(top) if top.canonicalize().ok() != here => top,
            _ => PathBuf::new(),
        }
    })
    .clone()
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
    static MAIN: OnceLock<PathBuf> = OnceLock::new();
    MAIN.get_or_init(|| {
        let root = root();
        rev_parse(&root, &["--path-format=absolute", "--git-common-dir"])
            .and_then(|common| common.parent().map(Path::to_path_buf))
            .unwrap_or(root)
    })
    .clone()
}

/// The checkout holding [`root`] — a worktree's own top level, not the
/// main checkout's — for commands that read or write committed files.
/// Outside git, [`root`] itself.
#[must_use]
pub fn toplevel() -> PathBuf {
    static TOP: OnceLock<PathBuf> = OnceLock::new();
    TOP.get_or_init(|| {
        let root = root();
        rev_parse(&root, &["--show-toplevel"]).unwrap_or(root)
    })
    .clone()
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
