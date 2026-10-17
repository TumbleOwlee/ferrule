use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use ferrule_core::attribution::{self, Rewrite};
use ferrule_core::config::Config;
use ferrule_core::{guard, project};
use serde::Deserialize;
use serde_json::{Value, json};

use super::exit;
use crate::cli::HookCommand;

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
        HookCommand::PreToolUse => pre_tool_use(cmd, &cwd, &payload.tool_input),
        HookCommand::Reviewer => reviewer(cmd, &cwd),
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

fn pre_tool_use(cmd: &str, cwd: &Path, tool_input: &Value) -> Option<Verdict> {
    if let Some(reason) = guard::shell(cmd, cwd) {
        return Some(Verdict::Deny(reason));
    }
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
