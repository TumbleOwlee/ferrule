use std::io::{self, Write};

use clap::Args;
use ferrule_core::install::{self, Error, Options, Stack, VERSION};
use ferrule_core::project;

use super::exit;
use crate::cli::Command;
use crate::commands::hook::HookCommand;

#[derive(Debug, Args)]
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
