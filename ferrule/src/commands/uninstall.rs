use std::io::{self, Write};

use ferrule_core::{install, project};

use super::exit;

/// Shown after `ferrule setup uninstall --help`.
pub const UNINSTALL_AFTER_LONG_HELP: &str = "\
Output:
  uninstall=done removed=<n> kept=<n>
                                 Counts of what was taken out and what was
                                 left. Removed: unedited generated files,
                                 ferrule's marked blocks, its settings
                                 entries and git hooks, the CLAUDE.md line.
                                 Deny rules you already had stay: only the
                                 ones install added are taken out.
  kept=<path> reason=edited      A generated file someone edited stays;
                                 delete it by hand if unwanted.
Refusals (stdout, exit 1):
  uninstall=refused reason=<error> hint=fix the file the reason names, then rerun
                                 The lock or .claude/settings.json is
                                 unreadable or not valid.
Exit codes: 0 ok, 1 refused, 2 usage, 4 internal error.";

pub fn run(purge: bool, out: &mut impl Write) -> io::Result<u8> {
    match install::uninstall(&project::toplevel(), purge) {
        Ok(r) => {
            writeln!(
                out,
                "uninstall=done removed={} kept={}",
                r.removed.len(),
                r.kept.len()
            )?;
            for k in &r.kept {
                writeln!(out, "kept={k} reason=edited")?;
            }
            Ok(exit::OK)
        }
        Err(e) => {
            writeln!(
                out,
                "uninstall=refused reason={e} hint=fix the file the reason names, then rerun"
            )?;
            Ok(exit::REFUSED)
        }
    }
}
