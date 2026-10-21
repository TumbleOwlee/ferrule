//! The git hooks install writes into the repository's hooks directory,
//! keeping a user's own hook it finds there.

use std::path::{Path, PathBuf};

use crate::git;

use super::items::{foreign_extends, lefthook};
use super::{GENERATED, Hooks, Report, VERSION, set_exec};

pub(super) const GIT_HOOKS: &[&str] = &["pre-commit", "commit-msg", "pre-push"];

/// Where git runs this repository's hooks, and whether that is inside its
/// git directory: a `core.hooksPath` elsewhere (a global one) is shared
/// with other repositories.
pub(super) fn hooks_dir(root: &Path) -> Option<(PathBuf, bool)> {
    let dir = root.join(git::out(root, &["rev-parse", "--git-path", "hooks"])?);
    let common = root.join(git::out(root, &["rev-parse", "--git-common-dir"])?);
    let own = match (dir.canonicalize(), common.canonicalize()) {
        (Ok(d), Ok(c)) => d.starts_with(c),
        // Not there yet: git creates `hooks` under its own directory.
        (Err(_), Ok(c)) => dir.parent().and_then(|p| p.canonicalize().ok()) == Some(c),
        _ => false,
    };
    Some((dir, own))
}

/// A git hook calling the shim; a checkout without it (another repository
/// sharing the hooks directory) is let through.
fn git_hook_text(name: &str) -> String {
    format!(
        "#!/bin/sh\n# {GENERATED} {VERSION}, edit .ferrule/ instead\nshim=\"$(git rev-parse --show-toplevel)/.ferrule/ferrule\"\n[ -x \"$shim\" ] || exit 0\nexec \"$shim\" hook git {name} \"$@\"\n"
    )
}

/// The git hooks: through lefthook's configuration (an install item), in
/// the hooks directory, or left to the user.
pub(super) fn ensure_git_hooks(root: &Path, report: &mut Report) -> std::io::Result<()> {
    if let Some(file) = lefthook(root) {
        if foreign_extends(root, file) {
            report.hooks = Some(Hooks::Manual);
            report.hints.push(format!("hint=add .ferrule/lefthook.yml to the extends list of {file}, then run lefthook install"));
        } else {
            report.hooks = Some(Hooks::Lefthook);
            report
                .hints
                .push("hint=run lefthook install so git runs the ferrule hooks".to_owned());
        }
        return Ok(());
    }
    if root.join(".husky").is_dir() {
        report.hooks = Some(Hooks::Manual);
        for name in GIT_HOOKS {
            report.hints.push(format!(
                "hint=add to .husky/{name}: .ferrule/ferrule hook git {name} \"$@\""
            ));
        }
        return Ok(());
    }
    let Some((dir, own)) = hooks_dir(root) else {
        report.hooks = Some(Hooks::Manual);
        report
            .hints
            .push("hint=not a git repository: no git hooks written".to_owned());
        return Ok(());
    };
    if !own {
        report.hooks = Some(Hooks::Manual);
        for name in GIT_HOOKS {
            report.hints.push(format!(
                "hint=core.hooksPath points outside the repository ({}): add to its {name}: .ferrule/ferrule hook git {name} \"$@\"",
                dir.display()
            ));
        }
        return Ok(());
    }
    std::fs::create_dir_all(&dir)?;
    for name in GIT_HOOKS {
        let path = dir.join(name);
        let text = git_hook_text(name);
        // Bytes, not text: a hook of the user's need not be UTF-8, and one
        // that is not must be kept, not taken for absent.
        match std::fs::read(&path) {
            Ok(old) if old == text.as_bytes() => {}
            Ok(old) if !String::from_utf8_lossy(&old).contains(GENERATED) => {
                report.hints.push(format!("hint=kept your .git/hooks/{name}; call .ferrule/ferrule hook git {name} \"$@\" from it"));
            }
            Ok(_) => {
                std::fs::write(&path, &text)?;
                set_exec(&path)?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                std::fs::write(&path, &text)?;
                set_exec(&path)?;
            }
            Err(e) => return Err(e),
        }
    }
    report.hooks = Some(Hooks::Git);
    Ok(())
}
