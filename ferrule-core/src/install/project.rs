//! The project's own files install sets up once: ferrule.toml with the
//! detected stack and main branch, and CLAUDE.md's import of AGENTS.md.

use std::fmt::Write as _;
use std::path::Path;

use crate::{config, git};

use super::lock::{Lock, quote};
use super::{CONFIG, Error, Options, Report, Stack, VERSION, write};

/// `origin/HEAD`'s branch, else git's `init.defaultBranch`, else `main`.
fn main_branch(root: &Path) -> String {
    git::out(
        root,
        &["symbolic-ref", "--short", "-q", "refs/remotes/origin/HEAD"],
    )
    .and_then(|r| r.strip_prefix("origin/").map(str::to_owned))
    .or_else(|| git::out(root, &["config", "--get", "init.defaultBranch"]))
    .filter(|b| !b.is_empty())
    .unwrap_or_else(|| "main".to_owned())
}

/// `text` with its top-level `version` set to `line`, every other line and
/// line ending as it was: a `version` key inside a table is not the pin.
pub(super) fn pin_version(text: &str, line: &str) -> String {
    let nl = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let mut out = String::with_capacity(text.len() + line.len());
    let mut top = true;
    let mut done = false;
    for l in text.split_inclusive('\n') {
        let bare = l.trim_end_matches(['\n', '\r']);
        let key = bare.trim_start();
        if key.starts_with('[') {
            top = false;
        }
        let is_pin = top
            && !done
            && key
                .strip_prefix("version")
                .is_some_and(|r| r.trim_start().starts_with('='));
        if is_pin {
            out.push_str(line);
            out.push_str(&l[bare.len()..]);
            done = true;
        } else {
            out.push_str(l);
        }
    }
    if done {
        out
    } else {
        format!("{line}{nl}{text}")
    }
}

fn on_path(program: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(program).is_file()))
}

/// A new project's `ferrule.toml`.
pub(super) fn config_text(stack: Stack, main: &str, viewer: bool) -> String {
    let mut out = format!(
        "version = {}\nstack = {}\nmain_branch = {}\n\n[backend]\ntracker = \"github\"\nhost = \"github\"\n",
        quote(VERSION),
        quote(stack.as_str()),
        quote(main)
    );
    for (name, cmd) in stack.gauntlet() {
        let _ = write!(
            out,
            "\n[[gauntlet.step]]\nname = \"{name}\"\ncmd = {cmd}\ntimeout = \"30m\"\n"
        );
    }
    if viewer {
        out.push_str("\n[viewer]\ncommand = [\"glow\"]\n");
    }
    out
}

/// `ferrule.toml` created, or its `version` moved to this binary's; the
/// viewer added only where the file has none.
pub(super) fn ensure_config(root: &Path, opts: &Options, report: &mut Report) -> Result<(), Error> {
    let path = root.join(CONFIG);
    let glow = on_path("glow");
    match std::fs::read_to_string(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let stack = opts.stack.unwrap_or_else(|| Stack::detect(root));
            report.stack = Some(stack);
            report.viewer = glow;
            write(
                root,
                CONFIG,
                &config_text(stack, &main_branch(root), glow),
                false,
            )?;
            report.written.push(CONFIG.to_owned());
        }
        Err(e) => return Err(e.into()),
        Ok(text) => {
            let parsed = config::Config::load(root).ok();
            report.stack = parsed
                .as_ref()
                .and_then(|c| c.stack.as_deref())
                .and_then(Stack::parse);
            let has_viewer = text.lines().any(|l| l.trim() == "[viewer]");
            let mut new = pin_version(&text, &format!("version = {}", quote(VERSION)));
            if !has_viewer && glow {
                let nl = if text.contains("\r\n") { "\r\n" } else { "\n" };
                if !new.ends_with('\n') {
                    new.push_str(nl);
                }
                for part in [nl, "[viewer]", nl, "command = [\"glow\"]", nl] {
                    new.push_str(part);
                }
            }
            report.viewer = has_viewer && parsed.is_some_and(|c| c.viewer.command.is_some())
                || (!has_viewer && glow);
            if new != text {
                std::fs::write(&path, new)?;
                report.written.push(CONFIG.to_owned());
            }
        }
    }
    Ok(())
}

pub(super) fn ensure_claude_md(
    root: &Path,
    lock: &mut Lock,
    report: &mut Report,
) -> std::io::Result<()> {
    let path = root.join("CLAUDE.md");
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    if text
        .lines()
        .any(|l| matches!(l.trim(), "@AGENTS.md" | "@./AGENTS.md"))
    {
        return Ok(());
    }
    let sep = if text.is_empty() || text.ends_with("\n\n") {
        ""
    } else if text.ends_with('\n') {
        "\n"
    } else {
        "\n\n"
    };
    std::fs::write(&path, format!("{text}{sep}@AGENTS.md\n"))?;
    lock.claude_md = true;
    report.written.push("CLAUDE.md".to_owned());
    Ok(())
}
