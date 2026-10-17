use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use ferrule_core::attribution::{self, Rewrite};
use ferrule_core::board::{self, State};
use ferrule_core::config::Config;
use ferrule_core::{git, guard, project};
use serde::Deserialize;
use serde_json::{Value, json};

use super::exit;
use crate::cli::{GitHook, HookCommand};

/// The part of Claude Code's `PreToolUse` payload the guards read.
#[derive(Debug, Deserialize)]
struct Payload {
    tool_name: String,
    /// Kept whole: a rewrite hands back every field, changed or not.
    #[serde(default)]
    tool_input: Value,
    cwd: Option<PathBuf>,
}

/// Exit 2 blocks the tool call with stderr as the reason; a guard that
/// cannot read its payload refuses instead of letting the call through.
pub fn run(command: &HookCommand, input: &mut impl Read, out: &mut impl Write) -> io::Result<u8> {
    match command {
        HookCommand::SessionStart => return session_start(out),
        HookCommand::Git { hook, args } => return git_hook(*hook, args, input),
        _ => {}
    }
    let mut raw = String::new();
    input.read_to_string(&mut raw)?;
    let payload: Payload = match serde_json::from_str(&raw) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("ferrule hook: unreadable PreToolUse payload ({e}); refusing the call");
            return Ok(exit::USAGE);
        }
    };
    if payload.tool_name != "Bash" {
        return Ok(exit::OK);
    }
    let Some(cmd) = payload.tool_input.get("command").and_then(Value::as_str) else {
        return Ok(exit::OK);
    };
    let cwd = payload.cwd.clone().unwrap_or_else(|| PathBuf::from("."));
    let verdict = match command {
        HookCommand::Shell => shell(cmd, &cwd),
        HookCommand::Attribution => attribution(cmd, &cwd, &payload.tool_input),
        HookCommand::Reviewer => reviewer(cmd, &cwd),
        HookCommand::SessionStart | HookCommand::Git { .. } => None,
    };
    let output = match verdict {
        None => return Ok(exit::OK),
        Some(Verdict::Deny(reason)) => json!({
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "deny",
                "permissionDecisionReason": reason,
            }
        }),
        Some(Verdict::Rewrite(input)) => json!({
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "updatedInput": input,
                "additionalContext": "ferrule dropped a tool-attribution trailer from this command; this repo never carries one.",
            }
        }),
    };
    writeln!(out, "{output}")?;
    Ok(exit::OK)
}

enum Verdict {
    Deny(String),
    /// The whole tool input with the command replaced.
    Rewrite(Value),
}

/// The reviewer's guard with the project's extra mutating commands; a
/// config it cannot read refuses the call rather than guess.
fn reviewer(cmd: &str, cwd: &Path) -> Option<Verdict> {
    match Config::load(&project::main_checkout()) {
        Ok(config) => guard::readonly(cmd, cwd, &config.hook.readonly.mutating).map(Verdict::Deny),
        Err(e) => Some(Verdict::Deny(format!("ferrule hook reviewer: {e}"))),
    }
}

/// The command guard with the project's `[guard] allow`; a config it
/// cannot read refuses the call rather than guess.
fn shell(cmd: &str, cwd: &Path) -> Option<Verdict> {
    match Config::load(&project::main_checkout()) {
        Ok(config) => guard::shell(
            cmd,
            cwd,
            guard::Policy {
                allow: &config.guard.allow,
            },
        )
        .map(Verdict::Deny),
        Err(e) => Some(Verdict::Deny(format!("ferrule hook shell: {e}"))),
    }
}

fn attribution(cmd: &str, cwd: &Path, tool_input: &Value) -> Option<Verdict> {
    match attribution::rewrite(cmd, cwd) {
        Rewrite::Clean => None,
        Rewrite::InFile(path) => Some(Verdict::Deny(format!(
            "{path} carries a tool-attribution trailer (Co-Authored-By / \"Generated with\"), which this repo never carries. Remove those lines from the file and retry."
        ))),
        Rewrite::Stripped(clean) => {
            let mut input = tool_input.clone();
            input["command"] = Value::String(clean);
            Some(Verdict::Rewrite(input))
        }
    }
}

/// Unfinished runs: any card in inprogress or inreview. Silent otherwise.
fn session_start(out: &mut impl Write) -> io::Result<u8> {
    let Ok(Some(s)) = super::card::store(false) else {
        return Ok(exit::OK);
    };
    let Ok(cards) = board::all(&s.conn) else {
        return Ok(exit::OK);
    };
    let open: Vec<_> = cards
        .iter()
        .filter(|(_, st)| matches!(st, State::InProgress | State::InReview))
        .collect();
    if open.is_empty() {
        return Ok(exit::OK);
    }
    writeln!(out, "[ferrule] unfinished cards — a run was interrupted:")?;
    for (id, st) in open {
        writeln!(out, "{id} {}", st.as_str())?;
    }
    writeln!(
        out,
        "Ask to resume: reconcile every card against git before acting on it (ferrule guide resume)."
    )?;
    Ok(exit::OK)
}

/// Exit 1 refuses the git operation, with the reason on stderr.
fn git_hook(hook: GitHook, args: &[String], input: &mut impl Read) -> io::Result<u8> {
    let root = project::main_checkout();
    let main = Config::load(&root).map_or_else(|_| "main".to_owned(), |c| c.main_branch);
    match hook {
        GitHook::PreCommit => {
            let here = std::env::current_dir()?;
            if git::out(&here, &["symbolic-ref", "--short", "-q", "HEAD"]).as_deref()
                == Some(main.as_str())
            {
                eprintln!(
                    "ferrule: commit on '{main}' refused: branch off it, never commit to it."
                );
                return Ok(exit::REFUSED);
            }
            Ok(exit::OK)
        }
        GitHook::CommitMsg => {
            let Some(file) = args.first() else {
                eprintln!("ferrule hook git commit-msg: the message file is missing");
                return Ok(exit::USAGE);
            };
            let text = std::fs::read_to_string(file)?;
            if let Some(clean) = attribution::strip(&text) {
                std::fs::write(file, clean)?;
                eprintln!("ferrule: dropped a tool-attribution trailer from the commit message.");
            }
            Ok(exit::OK)
        }
        GitHook::PrePush => {
            let mut refs = String::new();
            input.read_to_string(&mut refs)?;
            let target = format!("refs/heads/{main}");
            if refs
                .lines()
                .any(|l| l.split_whitespace().nth(2) == Some(target.as_str()))
            {
                eprintln!("ferrule: push to '{main}' refused: merge to it by pull request only.");
                return Ok(exit::REFUSED);
            }
            Ok(exit::OK)
        }
    }
}
