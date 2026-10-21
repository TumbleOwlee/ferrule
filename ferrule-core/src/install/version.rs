//! Version policy: which ferrule a project wants, against this binary,
//! and the one command a mismatch still lets through, the update.

use std::path::Path;

use super::{LOCK, Lock, VERSION};
use crate::config;
use crate::refusal::Refusal;

/// A version the project records that this binary is not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mismatch {
    /// `version` in `ferrule.toml`.
    Pinned(String),
    /// `version` in `ferrule.lock`: the binary that wrote the generated
    /// files.
    Locked(String),
}

/// `a` is a later `x.y.z` than `b`; false when either does not parse.
fn later(a: &str, b: &str) -> bool {
    let parse = |v: &str| -> Option<Vec<u64>> { v.split('.').map(|n| n.parse().ok()).collect() };
    matches!((parse(a), parse(b)), (Some(a), Some(b)) if a > b)
}

impl Mismatch {
    /// The status line that names the update the project needs.
    #[must_use]
    pub fn message(&self) -> String {
        let (key, v) = match self {
            Mismatch::Pinned(v) => ("pinned", v),
            Mismatch::Locked(v) => ("locked", v),
        };
        let cargo = format!("cargo install --git {REPO} --tag v{v} --locked");
        let fix = if later(v, VERSION) {
            format!("this binary is older than the project's; run {cargo}")
        } else {
            format!(
                "the project's ferrule files are from {v}; run ferrule setup install to update them to {VERSION} and commit the result, or keep {v} with {cargo}"
            )
        };
        Refusal::new("ferrule", "version")
            .field(key, v)
            .field("binary", VERSION)
            .hint(fix)
            .to_string()
    }
}

/// The repository `cargo install --git` fetches ferrule from.
pub const REPO: &str = "https://github.com/TumbleOwlee/ferrule";

/// The version the project pins or its generated files were written by,
/// when this binary is not it; the pin is named first.
#[must_use]
pub fn version_mismatch(root: &Path) -> Option<Mismatch> {
    if let Some(pinned) = config::Config::load(root).ok().and_then(|c| c.version)
        && pinned != VERSION
    {
        return Some(Mismatch::Pinned(pinned));
    }
    if !root.join(LOCK).is_file() {
        return None;
    }
    let locked = Lock::load(root).ok()?.version;
    (!locked.is_empty() && locked != VERSION).then_some(Mismatch::Locked(locked))
}

/// `command` installs ferrule or runs `ferrule setup install` (or the old
/// `ferrule install`, gone with `legacy.rs`): what a version mismatch asks
/// for, so the guard lets it through.
#[must_use]
pub fn is_update(command: &str) -> bool {
    let words: Vec<&str> = command.split_whitespace().collect();
    // `ferrule` or a path to it, never a word that only ends in it.
    let is_ferrule = |w: &str| w == "ferrule" || w.ends_with("/ferrule");
    let ferrule_install = words
        .windows(2)
        .any(|w| is_ferrule(w[0]) && w[1] == "install")
        || words
            .windows(3)
            .any(|w| is_ferrule(w[0]) && w[1] == "setup" && w[2] == "install");
    let cargo_install = words.windows(2).any(|w| w == ["cargo", "install"])
        && words
            .iter()
            .any(|w| is_ferrule(w.trim_end_matches(".git")) || w.starts_with("ferrule@"));
    ferrule_install || cargo_install
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ut_versions_compare_and_updates_are_recognized() {
        assert!(later("0.10.0", "0.9.3"));
        assert!(!later("0.9.3", "0.10.0"));
        assert!(!later("dev", "0.1.0"));
        assert!(is_update(".ferrule/ferrule setup install --force"));
        assert!(
            is_update("ferrule install"),
            "the old name, kept for one release"
        );
        assert!(!is_update("ferrule setup check"));
        assert!(is_update(
            "cargo install --git https://github.com/TumbleOwlee/ferrule --tag v1.0.0"
        ));
        assert!(!is_update("ferrule board list"));
        assert!(!is_update("cargo install ripgrep"));
        assert!(!is_update("notferrule install"));
        assert!(!is_update("cargo install ripgrep-ferrule-fork"));
        assert!(is_update("cargo install --path ferrule"));
        assert!(is_update(
            "cargo install --git https://github.com/o/ferrule.git"
        ));
    }
}
