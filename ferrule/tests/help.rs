//! Help coverage: every `key=value` status line the code can print is
//! documented in the `--help` of the command that prints it. The scanner reads
//! source text, so a new status line without a doc fails the group's test.

mod golden;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use golden::Run;

/// Source file to the commands whose `--help` documents its status tokens.
const SOURCES: &[(&str, &[&str])] = &[
    ("ferrule-core/src/body.rs", &["issue body", "pr body"]),
    ("ferrule-core/src/card/board.rs", &["board move"]),
    ("ferrule-core/src/card/vocab.rs", &["board log"]),
    ("ferrule-core/src/guide.rs", &["guide show", "guide check"]),
    (
        "ferrule-core/src/install/mod.rs",
        &["setup install", "setup check"],
    ),
    (
        "ferrule-core/src/pr/publish.rs",
        &[
            "issue post",
            "pr open",
            "pr push",
            "pr publish",
            "pr feedback route",
        ],
    ),
    ("ferrule-core/src/prompt/render.rs", &["prompt render"]),
    (
        "ferrule-core/src/run/archive.rs",
        &["run metrics", "run cleanup"],
    ),
    (
        "ferrule-core/src/run/gate.rs",
        &["run init", "gate approve", "stage replan"],
    ),
    ("ferrule-core/src/run/gauntlet.rs", &["ci gauntlet"]),
    ("ferrule-core/src/run/mod.rs", &["stage stop"]),
    ("ferrule-core/src/run/next.rs", &["gate next"]),
    ("ferrule-core/src/run/rebuild.rs", &["run rebuild"]),
    (
        "ferrule/src/commands/board.rs",
        &[
            "board events",
            "board import",
            "board list",
            "board log",
            "board move",
            "board new",
            "board set",
            "board show",
            "board vocab",
            "stage commit",
        ],
    ),
    (
        "ferrule/src/commands/ci.rs",
        &["ci gauntlet", "ci wait", "ci failed"],
    ),
    (
        "ferrule/src/commands/file.rs",
        &["file headings", "file section"],
    ),
    (
        "ferrule/src/commands/gate.rs",
        &["gate approve", "gate next"],
    ),
    (
        "ferrule/src/commands/guide.rs",
        &["guide show", "guide list", "guide check"],
    ),
    (
        "ferrule/src/commands/install.rs",
        &["setup install", "setup check"],
    ),
    (
        "ferrule/src/commands/issue.rs",
        &[
            "issue body",
            "issue post",
            "issue search",
            "issue view",
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
        &["pr open", "pr push", "pr publish", "ci failed"],
    ),
    (
        "ferrule/src/commands/run.rs",
        &[
            "run cleanup",
            "run diff",
            "run export",
            "run init",
            "run log",
            "run metrics",
            "run plan-diff",
            "run rebuild",
            "run reconcile",
            "run status",
            "gate approve",
            "gate next",
            "stage verify",
            "stage stop",
            "stage replan",
            "ci gauntlet",
            "ci wait",
        ],
    ),
    (
        "ferrule/src/commands/stage.rs",
        &["stage commit", "stage verify", "stage stop", "stage replan"],
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
        &["spec collisions", "spec next", "spec refs", "spec related"],
    ),
    ("ferrule/src/commands/uninstall.rs", &["setup uninstall"]),
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
        "ferrule-core/src/refusal.rs",
        "the refusal renderer's own format; each verb documents its refusals",
    ),
    (
        "ferrule/src/cli.rs",
        "the root help's own description of status lines and refusals",
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

/// The `"..."` literal at the start of `s`, after whitespace; `None` when
/// `s` starts with anything else (a variable).
fn leading_literal(s: &str) -> Option<&str> {
    let rest = s.trim_start().strip_prefix('"')?;
    rest.find('"').map(|end| &rest[..end])
}

/// The builder chain of a `Refusal::new(` whose arguments start `s`: up to
/// the `;`, `,` or closing bracket that ends the expression, string
/// literals skipped.
fn statement(s: &str) -> &str {
    let mut in_str = false;
    let mut escaped = false;
    let mut depth = 1usize;
    for (i, c) in s.char_indices() {
        match c {
            _ if escaped => escaped = false,
            '\\' if in_str => escaped = true,
            '"' => in_str = !in_str,
            _ if in_str => {}
            '(' | '{' | '[' => depth += 1,
            ')' | '}' | ']' | ';' | ',' if depth == 0 => return &s[..i],
            ')' | '}' | ']' => depth -= 1,
            _ => {}
        }
    }
    s
}

/// The tokens of the refusals built in `code`: `Refusal::new("verb",
/// "reason")` is `verb=refused` (`verb=reject` when the chain calls
/// `.reject()`) and `reason=reason`; `Refusal::card("verb")` is
/// `verb=refused` and `reason=card`; a `.field("key", …)` in the chain is
/// `key=`. A verb or reason passed as a variable adds no token for it.
fn refusal_tokens(code: &str, found: &mut BTreeSet<String>) {
    for (at, _) in code.match_indices("Refusal::new(") {
        let args = &code[at + "Refusal::new(".len()..];
        let chain = statement(args);
        let verdict = if chain.contains(".reject()") {
            "reject"
        } else {
            "refused"
        };
        if let Some(verb) = leading_literal(args) {
            found.insert(format!("{verb}={verdict}"));
        }
        let second = args.split_once(',').map_or("", |(_, r)| r);
        if let Some(reason) = leading_literal(second) {
            found.insert(format!("reason={reason}"));
        }
        for (f, _) in chain.match_indices(".field(") {
            if let Some(key) = leading_literal(&chain[f + ".field(".len()..]) {
                found.insert(format!("{key}="));
            }
        }
    }
    for (at, _) in code.match_indices("Refusal::card(") {
        if let Some(verb) = leading_literal(&code[at + "Refusal::card(".len()..]) {
            found.insert(format!("{verb}=refused"));
        }
        found.insert("reason=card".to_owned());
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
    refusal_tokens(code, &mut found);
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
        vec!["board", "log"],
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
fn help_board() {
    check_group(&["board"]);
}

#[test]
fn help_run_gate_stage_ci() {
    check_group(&["run", "gate", "stage", "ci"]);
}

#[test]
fn help_spec_file_guide_setup() {
    check_group(&["spec", "file", "guide", "setup"]);
}

#[test]
fn help_issue_pr() {
    check_group(&["issue", "pr"]);
}

#[test]
fn help_prompt_hook() {
    check_group(&["prompt", "hook"]);
}

// Help to code: every refusal the long help documents can be produced.

/// Core files that build refusals and have no `SOURCES` entry of their own,
/// with the commands whose help documents them.
const REFUSAL_SOURCES: &[(&str, &[&str])] = &[
    (
        "ferrule-core/src/body.rs",
        &[
            "board move",
            "gate approve",
            "issue post",
            "pr open",
            "pr push",
            "stage replan",
        ],
    ),
    ("ferrule-core/src/card/vocab.rs", &["board move"]),
    (
        "ferrule-core/src/install/mod.rs",
        &["hook shell", "hook reviewer"],
    ),
    ("ferrule-core/src/pr/publish.rs", &["gate approve"]),
    ("ferrule-core/src/run/gauntlet.rs", &["pr push"]),
];

/// `(command, verb, reason)` entries that are documented but built from
/// runtime values the scanner cannot see, and why.
const DYNAMIC: &[(&str, &str, &str)] = &[];

/// Documented refusal lines that carry no `hint=` of their own, and why.
const NO_HINT: &[(&str, &str)] = &[];

/// Index of the `)` closing the call whose `(` is at `open`.
fn close_of(chars: &[char], open: usize) -> usize {
    let mut depth = 0;
    let mut i = open;
    while i < chars.len() {
        match chars[i] {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return i;
                }
            }
            '"' => {
                i += 1;
                while i < chars.len() && chars[i] != '"' {
                    i += usize::from(chars[i] == '\\') + 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    chars.len()
}

/// The arguments of a call at depth zero, trimmed.
fn args_of(text: &str) -> Vec<String> {
    let mut out = vec![String::new()];
    let (mut depth, mut quoted, mut escaped) = (0, false, false);
    for c in text.chars() {
        if quoted {
            quoted = c != '"' || escaped;
            escaped = c == '\\' && !escaped;
        } else {
            match c {
                '"' => quoted = true,
                '(' | '[' | '{' => depth += 1,
                ')' | ']' | '}' => depth -= 1,
                ',' if depth == 0 => {
                    out.push(String::new());
                    continue;
                }
                _ => {}
            }
        }
        out.last_mut().expect("one argument").push(c);
    }
    out.iter().map(|a| a.trim().to_string()).collect()
}

/// The text of a lone `"..."` literal argument.
fn literal_arg(arg: &str) -> Option<&str> {
    let inner = arg.strip_prefix('"')?.strip_suffix('"')?;
    (!inner.contains('"')).then_some(inner)
}

/// What the code can refuse with: `(verb, reason)` pairs from literal
/// `Refusal::new`/`Refusal::card` calls, and reasons alone where the verb is
/// a variable or the reason a typed enum's slug (`=> "reason"` arms).
#[derive(Default)]
struct Reasons {
    pairs: BTreeSet<(String, String)>,
    reasons: BTreeSet<String>,
}

fn scan_reasons(file: &str, into: &mut Reasons) {
    let src = std::fs::read_to_string(workspace().join(file)).expect("read source");
    let code = src.split("#[cfg(test)]").next().unwrap_or_default();
    let chars: Vec<char> = code.chars().collect();
    for (call, card) in [("Refusal::new(", false), ("Refusal::card(", true)] {
        let mut from = 0;
        while let Some(at) = code[from..].find(call) {
            let open = code[..from + at].chars().count() + call.chars().count() - 1;
            let end = close_of(&chars, open);
            let inner: String = chars[open + 1..end].iter().collect();
            let args = args_of(&inner);
            let verb = args.first().and_then(|a| literal_arg(a));
            let reason = if card {
                Some("card")
            } else {
                args.get(1).and_then(|a| literal_arg(a))
            };
            match (verb, reason) {
                (Some(v), Some(r)) => {
                    into.pairs.insert((v.to_string(), r.to_string()));
                }
                (None, Some(r)) => {
                    into.reasons.insert(r.to_string());
                }
                _ => {}
            }
            from += at + call.len();
        }
    }
    for text in code.lines() {
        if let Some(rest) = text.trim().split("=> \"").nth(1) {
            let word: String = rest.chars().take_while(|&c| is_key(c)).collect();
            if !word.is_empty() && rest[word.len()..].starts_with('"') {
                into.reasons.insert(word);
            }
        }
    }
}

/// A documented refusal: `<verb>=refused|reject reason=<a>|<b>`.
struct Documented {
    verb: String,
    reasons: Vec<String>,
}

fn documented(help: &str) -> Vec<Documented> {
    let mut out = Vec::new();
    for marker in ["=refused reason=", "=reject reason="] {
        for (at, _) in help.match_indices(marker) {
            let mut verb: Vec<char> = help[..at]
                .chars()
                .rev()
                .take_while(|&c| is_key(c))
                .collect();
            verb.reverse();
            let list: String = help[at + marker.len()..]
                .chars()
                .take_while(|&c| is_key(c) || c == '|')
                .collect();
            out.push(Documented {
                verb: verb.into_iter().collect(),
                reasons: list
                    .split('|')
                    .filter(|r| !r.is_empty())
                    .map(str::to_string)
                    .collect(),
            });
        }
    }
    out
}

/// Every command named in `SOURCES` or `REFUSAL_SOURCES`, sorted.
fn commands() -> BTreeSet<&'static str> {
    SOURCES
        .iter()
        .chain(REFUSAL_SOURCES)
        .flat_map(|&(_, cs)| cs.iter().copied())
        .collect()
}

#[test]
fn documented_refusals_are_producible() {
    let mut stale = Vec::new();
    for command in commands() {
        let mut found = Reasons::default();
        for &(file, cs) in SOURCES.iter().chain(REFUSAL_SOURCES) {
            if cs.contains(&command) {
                scan_reasons(file, &mut found);
            }
        }
        for d in documented(&help_of(command)) {
            for reason in &d.reasons {
                let known = found.pairs.contains(&(d.verb.clone(), reason.clone()))
                    || found.reasons.contains(reason)
                    || DYNAMIC
                        .iter()
                        .any(|&(c, v, r)| c == command && v == d.verb && r == reason);
                if !known {
                    stale.push(format!("{command}: {}=refused reason={reason}", d.verb));
                }
            }
        }
    }
    stale.sort();
    stale.dedup();
    assert!(
        stale.is_empty(),
        "documented refusals no source can produce:\n{}",
        stale.join("\n")
    );
}

/// Whether the entry on line `at` of `lines` is a literal line (not a
/// description column) and so shows its own `hint=`.
fn literal_entry(lines: &[&str], at: usize) -> bool {
    let line = lines[at].trim();
    let rest = line.split(" reason=").nth(1).unwrap_or_default();
    let after_reason = rest.split_once(' ').map_or("", |(_, r)| r);
    let fields_only = after_reason.split_whitespace().all(|t| t.contains('='));
    let continues = lines
        .get(at + 1)
        .is_some_and(|n| n.starts_with("      ") && !n.contains("=refused"));
    !rest.contains("  ") && !after_reason.is_empty() && fields_only && !continues
}

#[test]
fn documented_refusals_end_in_hint() {
    let mut bare = Vec::new();
    for command in commands() {
        let help = help_of(command);
        let lines: Vec<&str> = help.lines().collect();
        let mut entries = 0;
        for (i, line) in lines.iter().enumerate() {
            let line = line.trim();
            if !(line.contains("=refused reason=") || line.contains("=reject reason=")) {
                continue;
            }
            entries += 1;
            let listed = line.starts_with(is_key) && !line.contains("=done");
            if listed
                && literal_entry(&lines, i)
                && !line.contains("hint=")
                && !NO_HINT
                    .iter()
                    .any(|&(c, l)| c == command && line.starts_with(l))
            {
                bare.push(format!("{command}: {line}"));
            }
        }
        // Column entries state the rule in prose: `hint=` appears somewhere.
        if entries > 0 && !help.contains("hint=") {
            bare.push(format!("{command}: refusals documented, `hint=` never"));
        }
    }
    bare.sort();
    bare.dedup();
    assert!(
        bare.is_empty(),
        "refusal lines without `hint=`:\n{}",
        bare.join("\n")
    );
}
