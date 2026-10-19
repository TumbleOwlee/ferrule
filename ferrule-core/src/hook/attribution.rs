//! Tool-attribution trailers (`Co-Authored-By:`, "Generated with …") in
//! commit messages, PR and issue bodies and comments: found and dropped, so
//! a command carrying one goes through without it instead of failing.

use std::path::Path;

use crate::hook::shell::{self, Command};

/// Leading decoration a footer line may carry: list markers, emphasis,
/// quotes, blanks.
fn decoration(c: char) -> bool {
    matches!(c, '-' | '*' | '_' | '>' | ' ' | '\t')
}

/// Whether `line` is an attribution line.
#[must_use]
pub fn is_attribution(line: &str) -> bool {
    let lower = line.to_lowercase();
    let mut rest = lower.trim_start_matches(decoration);
    if let Some(r) = rest
        .strip_prefix('🤖')
        .or_else(|| rest.strip_prefix(":robot:"))
    {
        rest = r.trim_start_matches(decoration);
    }
    if rest.starts_with("co-authored-by:") || rest.starts_with("co-authored-by=") {
        return true;
    }
    rest.strip_prefix("generated with")
        .is_some_and(|r| r.chars().next().is_none_or(|c| !c.is_alphanumeric()))
}

/// `text` without its attribution lines and the blank lines they leave at
/// the end. Lines break at a newline or at a literal `\n`, which `gh`
/// passes through verbatim. `None` when nothing was dropped.
#[must_use]
pub fn strip(text: &str) -> Option<String> {
    let mut lines: Vec<&str> = Vec::new();
    let mut seps: Vec<&str> = Vec::new();
    let mut rest = text;
    loop {
        let next = [
            rest.find('\n').map(|i| (i, 1)),
            rest.find("\\n").map(|i| (i, 2)),
        ]
        .into_iter()
        .flatten()
        .min_by_key(|(i, _)| *i);
        let Some((i, len)) = next else {
            lines.push(rest);
            break;
        };
        lines.push(&rest[..i]);
        seps.push(&rest[i..i + len]);
        rest = &rest[i + len..];
    }
    if !lines.iter().any(|l| is_attribution(l)) {
        return None;
    }
    let mut kept: Vec<(&str, &str)> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if !is_attribution(line) {
            kept.push((line, seps.get(i).copied().unwrap_or("")));
        }
    }
    while kept.last().is_some_and(|(l, _)| l.trim().is_empty()) {
        kept.pop();
    }
    let trailing = text.ends_with('\n');
    let mut out = String::new();
    for (i, (line, sep)) in kept.iter().enumerate() {
        out.push_str(line);
        if i + 1 < kept.len() {
            out.push_str(sep);
        }
    }
    if trailing && !out.is_empty() {
        out.push('\n');
    }
    Some(out)
}

/// Commands whose text ends up on a commit, PR, issue or comment.
fn carries_message(c: &Command) -> bool {
    let Some((name, args)) = shell::program(&c.words) else {
        return false;
    };
    let positional: Vec<&str> = args
        .iter()
        .map(String::as_str)
        .filter(|a| !a.starts_with('-'))
        .collect();
    match name {
        "git" => shell::git(args).is_some_and(|g| matches!(g.sub, "commit" | "tag" | "notes")),
        "gh" => matches!(
            positional.as_slice(),
            ["pr" | "issue", "create" | "edit" | "comment" | "review", ..]
                | ["release", "create" | "edit", ..]
        ),
        _ => false,
    }
}

fn single_quoted(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

fn is_trailer_option(w: &str) -> bool {
    w == "--trailer"
}

fn is_file_option(w: &str) -> bool {
    matches!(w, "-F" | "--file" | "--body-file")
}

/// The outcome of [`rewrite`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rewrite {
    /// No attribution anywhere.
    Clean,
    /// The command with every attribution line and `--trailer` dropped.
    Stripped(String),
    /// The attribution sits in a file the command reads; the file has to
    /// change, which a guard must not do.
    InFile(String),
}

/// `command` without its attribution: message words and heredoc bodies
/// re-quoted without the lines, `--trailer Co-authored-by…` options
/// removed. Everything else stays byte for byte.
#[must_use]
pub fn rewrite(command: &str, cwd: &Path) -> Rewrite {
    let chars: Vec<char> = command.chars().collect();
    let mut edits: Vec<(std::ops::Range<usize>, String)> = Vec::new();
    for c in shell::parse(command).iter().filter(|c| carries_message(c)) {
        if c.spans.len() != c.words.len() {
            continue;
        }
        let mut i = 0;
        while i < c.words.len() {
            let w = &c.words[i];
            if is_file_option(w)
                && let Some(path) = c.words.get(i + 1)
                && path != "-"
                && std::fs::read_to_string(cwd.join(path)).is_ok_and(|t| strip(&t).is_some())
            {
                return Rewrite::InFile(path.clone());
            }
            let trailer = if is_trailer_option(w) {
                c.words.get(i + 1).filter(|v| is_attribution(v)).map(|_| 2)
            } else {
                w.strip_prefix("--trailer=")
                    .filter(|v| is_attribution(v))
                    .map(|_| 1)
            };
            if let Some(n) = trailer {
                let end = c.spans[i + n - 1].end;
                let start = c.spans[i].start;
                // Take the blank before the option along with it.
                let start = (0..start)
                    .rev()
                    .take_while(|&k| matches!(chars[k], ' ' | '\t'))
                    .last()
                    .unwrap_or(start);
                edits.push((start..end, String::new()));
                i += n;
                continue;
            }
            if let Some(clean) = strip(w) {
                edits.push((c.spans[i].clone(), single_quoted(&clean)));
            }
            i += 1;
        }
        for r in &c.redirects {
            if let (Some(body), Some(span)) = (&r.body, &r.body_span)
                && let Some(clean) = strip(body)
            {
                edits.push((span.clone(), clean));
            }
        }
    }
    if edits.is_empty() {
        return Rewrite::Clean;
    }
    edits.sort_by_key(|(r, _)| std::cmp::Reverse(r.start));
    let mut out = chars;
    for (range, text) in edits {
        out.splice(range, text.chars());
    }
    Rewrite::Stripped(out.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ut_attribution_lines() {
        for line in [
            "Co-Authored-By: Someone <a@b.c>",
            "🤖 Generated with a tool",
            "- 🤖 Generated with a tool",
            "*Generated with a tool*",
            "🤖 *Generated with a tool*",
            "> Generated with a tool",
            "Co-authored-by=A",
            "Generated with",
        ] {
            assert!(is_attribution(line), "{line}");
        }
        for line in [
            "docs: images regenerated with images/capture.py",
            "fix: hook no longer matches generated with in prose",
            "Generated without a tool",
        ] {
            assert!(!is_attribution(line), "{line}");
        }
    }

    #[test]
    fn ut_strip_drops_lines_and_trailing_blanks() {
        assert_eq!(
            strip("fix: x\n\nbody\n\nCo-Authored-By: A <a@b>\n").as_deref(),
            Some("fix: x\n\nbody\n")
        );
        assert_eq!(strip("x\\n\\nCo-authored-by: A").as_deref(), Some("x"));
        assert_eq!(strip("fix: x"), None);
    }

    fn stripped(cmd: &str) -> String {
        match rewrite(cmd, Path::new(".")) {
            Rewrite::Stripped(s) => s,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn ut_rewrite_requotes_a_message_word() {
        assert_eq!(
            stripped("git commit -m \"fix: x\n\nCo-Authored-By: S <a@b.c>\" && git push"),
            "git commit -m 'fix: x' && git push"
        );
    }

    #[test]
    fn ut_rewrite_heredoc_body_in_place() {
        assert_eq!(
            stripped("git commit -F - <<'EOF'\nfix: it's x\n\n🤖 Generated with a tool\nEOF\n"),
            "git commit -F - <<'EOF'\nfix: it's x\nEOF\n"
        );
    }

    #[test]
    fn ut_rewrite_drops_trailer_options() {
        assert_eq!(
            stripped("git commit --trailer \"Co-Authored-By: X <a@b.c>\" -m x"),
            "git commit -m x"
        );
        assert_eq!(
            stripped("git commit --trailer=Co-authored-by:X -m x"),
            "git commit -m x"
        );
    }

    #[test]
    fn ut_rewrite_ignores_other_commands() {
        assert_eq!(
            rewrite("echo 'Co-Authored-By: x' > notes.txt", Path::new(".")),
            Rewrite::Clean
        );
    }
}
