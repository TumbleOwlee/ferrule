use std::io::{self, Write};

use ferrule_core::{install, project};

use super::exit;

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
            writeln!(out, "uninstall=refused reason={e}")?;
            Ok(exit::REFUSED)
        }
    }
}
