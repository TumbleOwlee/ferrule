use std::io::{self, Write};

use clap::Args;
use ferrule_core::install::{self, Error, Options, Stack, VERSION};
use ferrule_core::project;

use super::exit;
use crate::cli::Command;
use crate::commands::hook::HookCommand;

/// Shown after `ferrule install --help`.
const INSTALL_AFTER_LONG_HELP: &str = "\
Output:
  install=done version=<v> host=<h> stack=<s> viewer=<glow|stdout> hooks=<k> written=<n> kept=<n>
      Summary. stack is `unknown` when the existing ferrule.toml names none;
      hooks is lefthook, git or manual.
  written=<path>
      One line per file created or replaced.
  kept=<path> new=<path>.ferrule-new diff=+<a>/-<b>
      A file someone edited was kept; the new version is beside it. Merge
      it in, then delete the .ferrule-new file.
  hint=<text>
      What to do by hand: hint=run lefthook install; hint=add
      .ferrule/lefthook.yml to the extends list of the lefthook file;
      hint=add … to .husky/<hook>; hint=kept your .git/hooks/<hook> (call
      ferrule's hook from it); hint=not a git repository; hint=no viewer
      (install glow and rerun).
Refusals (stdout):
  install=refused reason=foreign path=<file> hint=--force overwrites it
      A generated path holds a file ferrule never wrote; nothing was
      written. Move it away or rerun with --force.
  install=refused reason=<error> hint=repair or delete the file the reason names, then rerun
      The lock or .claude/settings.json is unreadable or not valid.
With --check, nothing is written:
  install-check=ok version=<v>
      Files match the lock and this binary.
  install-check=drift count=<n> hint=run ferrule install, review the diff, commit
      Followed by one `drift path=<file> reason=<r>` line each. r is
      missing, stale (unedited, older), reason=edited (changed by hand, or
      settings.json deny rules differ) or version (then pinned=<v> or
      locked=<v>, and binary=<v>).
  install-check=refused reason=<error> hint=repair or delete the file the reason names, then rerun
      The lock or settings file is unreadable.
Version mismatch (stderr of every other command):
  ferrule=refused reason=version pinned=<v> binary=<v>: <fix>
  ferrule=refused reason=version locked=<v> binary=<v>: <fix>
      ferrule.toml pins (pinned) or the lock was written by (locked)
      another version than the binary. The line names the fix: run ferrule
      install and commit, or cargo install the pinned tag. install and
      uninstall still run.
Exit codes: 0 ok, 1 refused or drift, 2 usage (2 also blocks a git hook on a
version mismatch).";

#[derive(Debug, Args)]
#[command(after_long_help = INSTALL_AFTER_LONG_HELP)]
pub struct InstallArgs {
    /// The agent host to write files for.
    #[arg(long, default_value = "claude", value_parser = ["claude"])]
    pub host: String,
    /// The stack for a new `ferrule.toml` [default: detected].
    #[arg(long, value_parser = ["rust", "node", "go", "python", "none"])]
    pub stack: Option<String>,
    /// Write nothing; fail when the generated files drift from the lock or
    /// from this binary.
    #[arg(long, conflicts_with_all = ["stack", "force"])]
    pub check: bool,
    /// Overwrite files at generated paths that ferrule did not write.
    #[arg(long)]
    pub force: bool,
}

/// Refuses every command but install's when the project pins another
/// version or its generated files come from one. A git hook refuses with
/// exit 2, the code that blocks it; the Claude Code hooks check for
/// themselves, so that the update they ask for can still run.
pub fn pinned(command: &Command) -> Option<u8> {
    if matches!(command, Command::Install(_) | Command::Uninstall { .. }) {
        return None;
    }
    if matches!(command, Command::Hook(h) if !matches!(h, HookCommand::Git { .. })) {
        return None;
    }
    let mismatch = install::version_mismatch(&project::main_checkout())?;
    eprintln!("{}", mismatch.message());
    Some(if matches!(command, Command::Hook(_)) {
        exit::USAGE
    } else {
        exit::REFUSED
    })
}

pub fn install(args: &InstallArgs, out: &mut impl Write) -> io::Result<u8> {
    let root = project::toplevel();
    if args.check {
        return match install::check(&root) {
            Ok(drift) if drift.is_empty() => {
                writeln!(out, "install-check=ok version={VERSION}")?;
                Ok(exit::OK)
            }
            Ok(drift) => {
                writeln!(
                    out,
                    "install-check=drift count={} hint=run ferrule install, review the diff, commit",
                    drift.len()
                )?;
                for d in drift {
                    writeln!(out, "drift {d}")?;
                }
                Ok(exit::REFUSED)
            }
            Err(e) => {
                writeln!(
                    out,
                    "install-check=refused reason={e} hint=repair or delete the file the reason names, then rerun"
                )?;
                Ok(exit::REFUSED)
            }
        };
    }
    let opts = Options {
        stack: args.stack.as_deref().and_then(Stack::parse),
        force: args.force,
    };
    let report = match install::install(&root, &opts) {
        Ok(r) => r,
        Err(Error::Foreign(path)) => {
            writeln!(
                out,
                "install=refused reason=foreign path={path} hint=--force overwrites it"
            )?;
            return Ok(exit::REFUSED);
        }
        Err(e) => {
            writeln!(
                out,
                "install=refused reason={e} hint=repair or delete the file the reason names, then rerun"
            )?;
            return Ok(exit::REFUSED);
        }
    };
    writeln!(
        out,
        "install=done version={VERSION} host={} stack={} viewer={} hooks={} written={} kept={}",
        args.host,
        report.stack.map_or("unknown", Stack::as_str),
        if report.viewer { "glow" } else { "stdout" },
        report.hooks.map_or("manual", |h| h.as_str()),
        report.written.len(),
        report.kept.len()
    )?;
    for w in &report.written {
        writeln!(out, "written={w}")?;
    }
    for (path, side, diff) in &report.kept {
        writeln!(out, "kept={path} new={side} diff={diff}")?;
    }
    for h in &report.hints {
        writeln!(out, "{h}")?;
    }
    Ok(exit::OK)
}
