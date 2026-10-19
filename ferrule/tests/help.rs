//! Help coverage: every `key=value` status line the code can print is
//! documented in the `--help` of the command that prints it. The scanner reads
//! source text, so a new status line without a doc fails the group's test.

mod golden;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use golden::Run;

/// Source file to the commands whose `--help` documents its status tokens.
const SOURCES: &[(&str, &[&str])] = &[
    ("ferrule-core/src/card/board.rs", &["card move"]),
    ("ferrule-core/src/card/vocab.rs", &["card log"]),
    ("ferrule-core/src/guide.rs", &["guide"]),
    ("ferrule-core/src/install/mod.rs", &["install"]),
    (
        "ferrule-core/src/pr/publish.rs",
        &["pr open", "pr push", "pr publish", "pr feedback route"],
    ),
    ("ferrule-core/src/prompt/render.rs", &["prompt render"]),
    ("ferrule-core/src/run/archive.rs", &["run metrics"]),
    (
        "ferrule-core/src/run/gate.rs",
        &["run approve", "run replan"],
    ),
    ("ferrule-core/src/run/gauntlet.rs", &["run gauntlet"]),
    ("ferrule-core/src/run/mod.rs", &["run stage-stop"]),
    ("ferrule-core/src/run/next.rs", &["run next"]),
    ("ferrule-core/src/run/rebuild.rs", &["run rebuild"]),
    (
        "ferrule/src/commands/card.rs",
        &[
            "card commit",
            "card import",
            "card log",
            "card move",
            "card new",
            "card set",
            "card show",
        ],
    ),
    ("ferrule/src/commands/guide.rs", &["guide"]),
    ("ferrule/src/commands/install.rs", &["install"]),
    (
        "ferrule/src/commands/issue.rs",
        &[
            "issue body",
            "issue file",
            "pr body",
            "pr open",
            "pr push",
            "pr publish",
            "pr feedback route",
        ],
    ),
    (
        "ferrule/src/commands/pr/feedback.rs",
        &[
            "pr feedback fetch",
            "pr feedback reply",
            "pr feedback route",
        ],
    ),
    (
        "ferrule/src/commands/pr/mod.rs",
        &["pr open", "pr push", "pr publish"],
    ),
    (
        "ferrule/src/commands/run.rs",
        &[
            "run approve",
            "run ci",
            "run cleanup",
            "run export",
            "run init",
            "run metrics",
            "run next",
            "run rebuild",
            "run reconcile",
            "run replan",
            "run sha-check",
            "run stage-stop",
        ],
    ),
    (
        "ferrule/src/commands/hook/mod.rs",
        &["hook shell", "hook reviewer", "hook session-start"],
    ),
    (
        "ferrule/src/commands/prompt.rs",
        &["prompt render", "prompt lines", "prompt show"],
    ),
    (
        "ferrule/src/commands/spec.rs",
        &["spec collisions", "spec related"],
    ),
    ("ferrule/src/commands/uninstall.rs", &["uninstall"]),
];

/// Tokens that are not CLI output: a file, or `file:token`, and why.
const IGNORED: &[(&str, &str)] = &[
    (
        "ferrule-core/src/card/board.rs:gauntlet=pass",
        "matched in the card log, not printed",
    ),
    (
        "ferrule-core/src/pr/feedback.rs",
        "gh api arguments and GraphQL variables",
    ),
    (
        "ferrule-core/src/project.rs",
        "an argument to git, `--path-format=absolute`",
    ),
    (
        "ferrule-core/src/prompt/render.rs:gauntlet=pass",
        "matched in the card log, not printed",
    ),
    (
        "ferrule-core/src/prompt/render.rs:replan=pre-authorized",
        "matched in the card log, not printed",
    ),
    (
        "ferrule/src/cli.rs:key=value",
        "the root help's own description of status lines",
    ),
    (
        "ferrule-core/src/remote.rs",
        "query strings of the issue host's API calls",
    ),
    (
        "ferrule-core/src/run/archive.rs:kind=",
        "matched in the card log (`stop kind=`), not printed",
    ),
    (
        "ferrule-core/src/run/gauntlet.rs:path-format=absolute",
        "an argument to git",
    ),
    (
        "ferrule-core/src/run/next.rs:gate=gate4",
        "matched in the card log, not printed",
    ),
];

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn is_key(c: char) -> bool {
    c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'
}

/// The text inside every `"..."` literal of `src`, comments skipped.
fn literals(src: &str) -> Vec<String> {
    let chars: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '/' if chars.get(i + 1) == Some(&'/') => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            '\'' if chars.get(i + 1) == Some(&'"') && chars.get(i + 2) == Some(&'\'') => i += 3,
            '"' => {
                let hashes = chars[..i].iter().rev().take_while(|&&c| c == '#').count();
                let raw = i > hashes && chars[i - hashes - 1] == 'r';
                let mut text = String::new();
                i += 1;
                while i < chars.len() {
                    let c = chars[i];
                    if raw {
                        let closes = (1..=hashes).all(|k| chars.get(i + k) == Some(&'#'));
                        if c == '"' && closes {
                            i += hashes;
                            break;
                        }
                    } else if c == '\\' {
                        text.push(c);
                        i += 1;
                        if let Some(&next) = chars.get(i) {
                            text.push(next);
                        }
                        i += 1;
                        continue;
                    } else if c == '"' {
                        break;
                    }
                    text.push(c);
                    i += 1;
                }
                out.push(text);
                i += 1;
            }
            _ => i += 1,
        }
    }
    out
}

/// `key=word` and `key=` (a dynamic value) tokens in one literal.
fn tokens_in(text: &str, found: &mut BTreeSet<String>) {
    let chars: Vec<char> = text.chars().collect();
    for (at, _) in chars.iter().enumerate().filter(|&(_, &c)| c == '=') {
        let mut start = at;
        while start > 0 && is_key(chars[start - 1]) {
            start -= 1;
        }
        while start < at && !chars[start].is_ascii_lowercase() {
            start += 1;
        }
        let glued = start > 0 && (chars[start - 1].is_alphanumeric() || chars[start - 1] == '_');
        if start == at || glued {
            continue;
        }
        let key: String = chars[start..at].iter().collect();
        if chars.get(at + 1) == Some(&'{') {
            found.insert(format!("{key}="));
            continue;
        }
        let word: String = chars[at + 1..].iter().take_while(|&&c| is_key(c)).collect();
        if word.starts_with(|c: char| c.is_ascii_lowercase()) {
            found.insert(format!("{key}={word}"));
        }
    }
}

/// Status tokens in `file` (relative to the workspace) before its tests.
fn scan(file: &str) -> BTreeSet<String> {
    let src = std::fs::read_to_string(workspace().join(file)).expect("read source");
    let code = src.split("#[cfg(test)]").next().unwrap_or_default();
    let mut found = BTreeSet::new();
    for text in literals(code) {
        tokens_in(&text, &mut found);
    }
    found
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("read dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// Every source file of both crates, relative to the workspace, sorted.
fn all_sources() -> Vec<String> {
    let root = workspace().canonicalize().expect("workspace");
    let mut paths = Vec::new();
    rust_files(&root.join("ferrule/src"), &mut paths);
    rust_files(&root.join("ferrule-core/src"), &mut paths);
    let mut files: Vec<String> = paths
        .iter()
        .map(|p| p.strip_prefix(&root).unwrap().display().to_string())
        .collect();
    files.sort();
    files
}

fn ignored(file: &str, token: &str) -> bool {
    IGNORED
        .iter()
        .any(|&(key, _)| key == file || key == format!("{file}:{token}"))
}

/// Tokens of `file` that the help has to document.
fn wanted(file: &str) -> Vec<String> {
    scan(file)
        .into_iter()
        .filter(|t| !ignored(file, t))
        .collect()
}

fn help_of(command: &str) -> String {
    let mut args: Vec<&str> = command.split(' ').collect();
    args.push("--help");
    let out = Run::new(args).output();
    assert!(out.status.success(), "`ferrule {command} --help` failed");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn every_status_file_is_mapped() {
    let unmapped: Vec<String> = all_sources()
        .into_iter()
        .filter(|f| !wanted(f).is_empty() && !SOURCES.iter().any(|&(s, _)| s == f))
        .collect();
    assert!(
        unmapped.is_empty(),
        "files with status tokens but no SOURCES entry: {unmapped:?}"
    );
}

#[test]
fn dash_h_equals_long_help() {
    for command in [
        vec![],
        vec!["run"],
        vec!["card", "log"],
        vec!["pr", "feedback", "reply"],
    ] {
        let short = Run::new(command.iter().copied().chain(["-h"])).output();
        let long = Run::new(command.iter().copied().chain(["--help"])).output();
        assert_eq!(
            short.stdout, long.stdout,
            "`-h` differs from `--help` for {command:?}"
        );
    }
}

/// Every token of the group's files appears in its commands' `--help`.
fn check_group(groups: &[&str]) {
    let mut missing = Vec::new();
    for &(file, commands) in SOURCES {
        if !commands
            .iter()
            .any(|c| groups.contains(&c.split(' ').next().unwrap_or_default()))
        {
            continue;
        }
        let help: String = commands.iter().map(|c| help_of(c)).collect();
        for token in wanted(file) {
            if !help.contains(&token) {
                missing.push(format!("{file} ({}): {token}", commands.join(", ")));
            }
        }
    }
    assert!(
        missing.is_empty(),
        "{} status tokens missing from --help:\n{}",
        missing.len(),
        missing.join("\n")
    );
}

#[test]
fn help_card() {
    check_group(&["card"]);
}

#[test]
#[ignore = "filled by its group's task"]
fn help_run() {
    check_group(&["run"]);
}

#[test]
fn help_spec_guide_install() {
    check_group(&["spec", "guide", "install", "uninstall"]);
}

#[test]
fn help_issue_pr() {
    check_group(&["issue", "pr"]);
}

#[test]
fn help_prompt_hook() {
    check_group(&["prompt", "hook"]);
}
