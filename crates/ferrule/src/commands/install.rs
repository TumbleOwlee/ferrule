use std::io::{self, Write};

use ferrule_core::install::{self, Error, Options, Stack, VERSION};
use ferrule_core::project;

use super::exit;
use crate::cli::{Command, InstallArgs};

/// Refuses every command but install's when the project pins another
/// version. A hook refuses with exit 2, the code that blocks its call.
pub fn pinned(command: &Command) -> Option<u8> {
    if matches!(command, Command::Install(_) | Command::Uninstall { .. }) {
        return None;
    }
    let pinned = install::version_mismatch(&project::main_checkout())?;
    eprintln!(
        "ferrule=refused reason=version pinned={pinned} binary={VERSION}: run cargo install --git https://github.com/TumbleOwlee/ferrule --tag v{pinned} --locked, or ferrule install to move the project to {VERSION}"
    );
    Some(if matches!(command, Command::Hook(_)) {
        exit::USAGE
    } else {
        exit::REFUSED
    })
}

pub fn install(args: &InstallArgs, out: &mut impl Write) -> io::Result<u8> {
    let root = project::main_checkout();
    if args.check {
        return match install::check(&root) {
            Ok(drift) if drift.is_empty() => {
                writeln!(out, "install-check=ok version={VERSION}")?;
                Ok(exit::OK)
            }
            Ok(drift) => {
                writeln!(out, "install-check=drift count={}", drift.len())?;
                for d in drift {
                    writeln!(out, "drift {d}")?;
                }
                Ok(exit::REFUSED)
            }
            Err(e) => {
                writeln!(out, "install-check=refused reason={e}")?;
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
            writeln!(out, "install=refused reason={e}")?;
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

pub fn uninstall(purge: bool, out: &mut impl Write) -> io::Result<u8> {
    match install::uninstall(&project::main_checkout(), purge) {
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
