//! The guides, the installed agent files and the guard's replacement hints
//! name `ferrule` commands and shell commands; each must exist in the CLI
//! and pass the shell guard, so text cannot outlive what it points at.

mod golden;

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use ferrule_core::hook::guard::{self, Policy};
use golden::Run;

/// Programs whose backticked uses in the guides must pass the shell guard.
const PROGRAMS: &[&str] = &[
    "ferrule", "git", "gh", "cp", "mv", "rm", "cat", "grep", "rg", "sed", "ls", "cargo", "npm",
    "sh", "bash", "eval", "cd", "find", "xargs", "awk",
];

/// Counter-examples: commands the text tells an agent never to run.
const REFUSED: &[&str] = &["git add -A", "git commit", "git checkout"];

fn core() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../ferrule-core")
}

fn markdown(dir: &Path, out: &mut Vec<(String, String)>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            markdown(&path, out);
        } else if path.extension().is_some_and(|e| e == "md") {
            let name = path.strip_prefix(core()).unwrap().display().to_string();
            out.push((name, fs::read_to_string(&path).unwrap()));
        }
    }
}

/// Every guide topic and installed file, raw: lane and done tags hide no
/// text from a scan of the source.
fn sources() -> Vec<(String, String)> {
    let mut out = Vec::new();
    markdown(&core().join("guide"), &mut out);
    markdown(&core().join("install"), &mut out);
    assert!(out.len() > 30, "{} files", out.len());
    out
}

/// The backticked spans of `text`, and the lines inside fenced blocks.
fn spans(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut fence = false;
    for line in text.lines() {
        if line.starts_with("```") {
            fence = !fence;
        } else if fence {
            out.push(line.trim().to_owned());
        } else if !line.contains("``") {
            out.extend(
                line.split('`')
                    .skip(1)
                    .step_by(2)
                    .map(|s| s.trim().to_owned()),
            );
        }
    }
    out
}

/// A command path's `--help`, run once.
struct Help(HashMap<String, (String, Vec<String>)>);

impl Help {
    /// The help text and the subcommands of `path`.
    fn of(&mut self, path: &[&str]) -> &(String, Vec<String>) {
        self.0.entry(path.join(" ")).or_insert_with(|| {
            let out = Run::new(path.iter().copied().chain(["--help"])).output();
            let text = String::from_utf8_lossy(&out.stdout).into_owned();
            let subs = text
                .lines()
                .skip_while(|l| *l != "Commands:")
                .skip(1)
                .take_while(|l| !l.is_empty())
                .filter_map(|l| l.split_whitespace().next())
                .filter(|name| *name != "help")
                .map(str::to_owned)
                .collect();
            (text, subs)
        })
    }
}

fn is_word(t: &str) -> bool {
    t.chars().next().is_some_and(|c| c.is_ascii_lowercase())
        && t.chars().all(|c| c.is_ascii_lowercase() || c == '-')
}

/// The `--flag` a token names, if it does.
fn flag(token: &str) -> Option<&str> {
    let rest = token.trim_start_matches(['[', '(']).strip_prefix("--")?;
    let end = rest
        .find(|c: char| !(c.is_ascii_lowercase() || c == '-'))
        .unwrap_or(rest.len());
    (end > 0).then(|| &rest[..end])
}

/// Whether `help` lists `--name` as an option.
fn lists(help: &str, name: &str) -> bool {
    let flag = format!("--{name}");
    help.match_indices(&flag).any(|(i, _)| {
        !help[i + flag.len()..].starts_with(|c: char| c.is_ascii_lowercase() || c == '-')
    })
}

/// What is wrong with the `ferrule …` mentions in `text`.
fn check_mentions(help: &mut Help, text: &str, problems: &mut Vec<String>) {
    let mut rest = text;
    while let Some(i) = rest.find("ferrule ") {
        let before = rest[..i].chars().last();
        let after = &rest[i + "ferrule ".len()..];
        rest = after;
        if before.is_some_and(|c| c.is_alphanumeric() || "-_./".contains(c)) {
            continue;
        }
        let end = after.find("ferrule ").unwrap_or(after.len());
        let tokens: Vec<&str> = after[..end]
            .split_whitespace()
            .map(|t| t.trim_end_matches([',', ';', ')', '.']))
            .collect();
        let mut path: Vec<&str> = Vec::new();
        let mut n = 0;
        while let Some(t) = tokens.get(n) {
            if !help.of(&path).1.iter().any(|s| s == t) {
                break;
            }
            path.push(t);
            n += 1;
        }
        let at = path.join(" ");
        if let Some(t) = tokens.get(n)
            && is_word(t)
            && !help.of(&path).1.is_empty()
        {
            problems.push(format!("no subcommand `{t}` after `ferrule {at}`: {text}"));
            continue;
        }
        let text_help = help.of(&path).0.clone();
        for name in tokens[n..].iter().filter_map(|t| flag(t)) {
            if name != "help" && !lists(&text_help, name) {
                problems.push(format!("`ferrule {at}` has no --{name}: {text}"));
            }
        }
    }
}

#[test]
fn it_ferrule_commands_in_the_text_exist() {
    let mut help = Help(HashMap::new());
    let mut problems = Vec::new();
    for (name, text) in sources() {
        for span in spans(&text) {
            let mut found = Vec::new();
            check_mentions(&mut help, &span, &mut found);
            problems.extend(found.into_iter().map(|p| format!("{name}: {p}")));
        }
    }
    for r in guard::REPLACED {
        let mut found = Vec::new();
        check_mentions(&mut help, r.instead, &mut found);
        problems.extend(
            found
                .into_iter()
                .map(|p| format!("REPLACED {}: {p}", r.raw)),
        );
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn it_shell_commands_in_the_guides_pass_the_guard() {
    let dir = tempfile::tempdir().unwrap();
    let policy = Policy { allow: &[] };
    let mut problems = Vec::new();
    let mut checked = 0;
    for (name, text) in sources() {
        for span in spans(&text) {
            let first = span.split_whitespace().next().unwrap_or("");
            let command = span.replace(['<', '>'], "");
            if !PROGRAMS.contains(&first) || command.contains('…') {
                continue;
            }
            checked += 1;
            if REFUSED.contains(&span.as_str()) {
                continue;
            }
            if let Some(reason) = guard::shell(&command, dir.path(), policy) {
                problems.push(format!("{name}: `{span}` is refused: {reason}"));
            }
        }
    }
    assert!(checked > 100, "{checked} commands checked");
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn it_slash_commands_and_scripts_in_the_text_exist() {
    let skills: Vec<String> = fs::read_dir(core().join("install/claude/skills"))
        .unwrap()
        .map(|e| e.unwrap().path().file_stem().unwrap().display().to_string())
        .collect();
    assert!(!skills.is_empty());
    let mut problems = Vec::new();
    for (name, text) in sources() {
        for span in spans(&text) {
            let first = span.split_whitespace().next().unwrap_or("");
            if Path::new(first).extension().is_some_and(|e| e == "sh") {
                problems.push(format!(
                    "{name}: `{span}` runs a script ferrule does not ship"
                ));
            } else if let Some(s) = first.strip_prefix('/').filter(|s| is_word(s)) {
                // `/workflow` is another tool's skill, named to say it is replaced.
                if s != "workflow" && !skills.iter().any(|k| k == s) {
                    problems.push(format!("{name}: `{span}` is not an installed skill"));
                }
            }
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}
