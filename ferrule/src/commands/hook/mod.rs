mod git;

use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use clap::Subcommand;
use ferrule_core::card::board::{self, State};
use ferrule_core::config::Config;
use ferrule_core::hook::attribution::{self, Rewrite};
use ferrule_core::hook::guard;
use ferrule_core::install;
use ferrule_core::project;
use serde::Deserialize;
use serde_json::{Value, json};

use super::exit;
use git::{GIT_AFTER_LONG_HELP, GitHook};

/// Shown after `ferrule hook shell --help`.
const SHELL_AFTER_LONG_HELP: &str = "\
Called by Claude Code's settings (a `PreToolUse` hook on Bash), not by hand.
Reads the hook payload as JSON on stdin: tool_name, tool_input.command, cwd.
Output:
  (nothing)            exit 0; the call is allowed (also any tool but Bash)
  one JSON line        exit 0; permissionDecision \"deny\" with a reason that
                       names the ferrule command to run instead
Refuses:
  a write to a file only ferrule writes (state.db, ferrule.lock, ...),
                       by any glob, brace or case form, and removing or
                       moving .ferrule itself; git clean -x over it
  a flood: whole-file cat of a .md or large file, unfiltered git diff/show,
                       unfiltered find -type, a raw CI log
  a git or gh call a ferrule command replaces; the reason names it
                       (`[guard] allow` in ferrule.toml lets one through).
                       Commit-making calls count: merge, cherry-pick,
                       revert, am, commit-tree, send-pack, update-ref on a
                       branch. Seen behind wrappers (timeout, nice, env,
                       sudo, exec, xargs, find -exec, ...), in sh -c and
                       eval strings, by absolute path, behind gh -R, and
                       in git -c / git config / gh alias definitions
  what the guard cannot follow: ANSI-C quoting $'...', arithmetic ((...)),
                       a substitution inside ${...} or $((...)), a script
                       piped, heredoc'd or here-string'd into a shell,
                       source or eval, a variable or substitution where the
                       program or a git/gh subcommand goes, cd into a glob
  every call but `ferrule setup install` while the binary and the project's pin
                       or lock disagree: `ferrule=refused reason=version ...`
Errors (stderr):
  ferrule hook: unreadable PreToolUse payload (...); refusing the call
                       exit 2, which blocks the call; fix the hook command
Exit codes: 0 ok, 2 blocked or unreadable payload.";

/// Shown after `ferrule hook attribution --help`.
const ATTRIBUTION_AFTER_LONG_HELP: &str = "\
Called by Claude Code's settings (a `PreToolUse` hook on Bash), not by hand.
Reads the hook payload as JSON on stdin: tool_name, tool_input.command, cwd.
Output:
  (nothing)            exit 0; the command carries no attribution trailer
  one JSON line        exit 0; updatedInput is the command with the
                       Co-Authored-By / \"Generated with\" lines dropped
                       (every other tool_input field kept); a `$(cat <<EOF …)`
                       message body counts, as do gh release and git tag
Refuses:
  a command that commits or posts from a file carrying such a trailer:
                       deny with the file's path; remove those lines from
                       the file and retry
Errors (stderr):
  ferrule hook: unreadable PreToolUse payload (...); refusing the call
                       exit 2, which blocks the call
Exit codes: 0 ok, 2 blocked or unreadable payload.";

/// Shown after `ferrule hook reviewer --help`.
const REVIEWER_AFTER_LONG_HELP: &str = "\
Called by Claude Code's settings (a `PreToolUse` hook on Bash for the
fe-reviewer agent), not by hand.
Reads the hook payload as JSON on stdin: tool_name, tool_input.command, cwd.
Output:
  (nothing)            exit 0; the call is read-only, or only appends to
                       review.md or rewrites review.verdict.md
  one JSON line        exit 0; permissionDecision \"deny\" with the reason
Refuses:
  any command that would alter the repo, the working tree or the
                       filesystem, and the `[hook.readonly] mutating`
                       prefixes of ferrule.toml; record what you needed to
                       probe as a finding for the implementer instead
  every call but `ferrule setup install` while the binary and the project's pin
                       or lock disagree: `ferrule=refused reason=version ...`
  deny when ferrule.toml cannot be read: ferrule hook reviewer: <error>
Errors (stderr):
  ferrule hook: unreadable PreToolUse payload (...); refusing the call
                       exit 2, which blocks the call
Exit codes: 0 ok, 2 blocked or unreadable payload.";

/// Shown after `ferrule hook session-start --help`.
const SESSION_START_AFTER_LONG_HELP: &str = "\
Called by Claude Code's settings (a `SessionStart` hook), not by hand. Reads
no input.
Output (stdout, added to the session's context):
  [ferrule] <version message>
                       the binary and the project's pin or lock disagree;
                       follow the message
  [ferrule] unfinished cards — a run was interrupted:
  <card> <state>       one line per card in inprogress or inreview
  Ask to resume: reconcile every card against git before acting on it
  (ferrule guide show resume).
Prints nothing when there is no board or no unfinished card.
Exit codes: 0 always.
See: ferrule guide show resume";

#[derive(Debug, Subcommand)]
pub enum HookCommand {
    /// Guard every Bash call (`PreToolUse`).
    ///
    /// Refuses writes to files only ferrule writes, output floods, the git
    /// and gh calls a ferrule command replaces (naming that command), and
    /// shell constructs it cannot follow.
    #[command(after_long_help = SHELL_AFTER_LONG_HELP)]
    Shell,
    /// Strip attribution trailers from a Bash call (`PreToolUse`).
    #[command(after_long_help = ATTRIBUTION_AFTER_LONG_HELP)]
    Attribution,
    /// Keep the reviewer read-only (`PreToolUse`).
    ///
    /// Refuses every write except the run's review.md and
    /// review.verdict.md.
    #[command(after_long_help = REVIEWER_AFTER_LONG_HELP)]
    Reviewer,
    /// Name the runs an interrupted session left (`SessionStart`).
    #[command(after_long_help = SESSION_START_AFTER_LONG_HELP)]
    SessionStart,
    /// Run a git hook (`pre-commit`, `commit-msg`, `pre-push`).
    ///
    /// `pre-commit` and `pre-push` refuse the main branch; `commit-msg`
    /// strips attribution trailers from the message file.
    #[command(after_long_help = GIT_AFTER_LONG_HELP)]
    Git {
        /// The hook git is running.
        hook: GitHook,
        /// The arguments git passes the hook.
        args: Vec<String>,
    },
}

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
        HookCommand::Git { hook, args } => return git::run(*hook, args, input),
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
    // A version mismatch refuses every call but the update it asks for.
    let guards = matches!(command, HookCommand::Shell | HookCommand::Reviewer);
    let mismatch = (guards && !install::is_update(cmd))
        .then(|| install::version_mismatch(&project::main_checkout()))
        .flatten();
    let verdict = match command {
        _ if let Some(m) = &mismatch => Some(Verdict::Deny(m.message())),
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
    if let Some(m) = install::version_mismatch(&project::main_checkout()) {
        writeln!(out, "[ferrule] {}", m.message())?;
    }
    let Ok(Some(s)) = super::board::store(false) else {
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
        "Ask to resume: reconcile every card against git before acting on it (ferrule guide show resume)."
    )?;
    Ok(exit::OK)
}
