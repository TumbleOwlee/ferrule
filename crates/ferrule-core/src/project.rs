//! Where the project a command acts on lives.

use std::path::PathBuf;

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
