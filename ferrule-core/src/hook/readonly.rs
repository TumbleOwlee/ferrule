//! The reviewer's guard: an allowlist. A command passes when every program
//! in it is one of the read-only programs below in a read-only form, or
//! starts with a prefix the project allowed. Everything else is refused,
//! including wrappers, `-c` scripts and anything the lexer cannot follow.

use std::path::Path;

use crate::hook::guard::{
    cd_target, has, has_prefix, is_artifact, opaque_reason, readonly_git, tee_targets,
};
use crate::hook::shell::{self, RedirectKind};

/// Programs that read and never write, whatever their arguments; shell
/// control words, whose commands the lexer lists on their own, included.
const PLAIN: &[&str] = &[
    "ls", "cat", "head", "tail", "wc", "grep", "diff", "cmp", "stat", "pwd", "echo", "printf",
    "cut", "tr", "nl", "basename", "dirname", "realpath", "jq", "test", "[", "true", "false",
    "for", "case", "select", "done", "fi", "esac",
];

/// The ferrule commands that only read: `(group, subcommands)`.
const FERRULE_READS: &[(&str, &[&str])] = &[
    ("board", &["show", "list", "field", "events", "vocab"]),
    ("run", &["status", "diff", "log", "plan-diff"]),
    ("stage", &["verify"]),
    ("file", &["headings", "section"]),
    (
        "spec",
        &["id", "related", "next", "refs", "collisions", "render"],
    ),
    ("issue", &["view", "search", "request", "body"]),
    ("pr", &["view", "body"]),
    ("guide", &["show", "list", "check"]),
    ("prompt", &["lines"]),
];

/// A ferrule call that only reads: a subcommand of [`FERRULE_READS`]
/// after any `--format`, or help.
fn ferrule_reads(args: &[String]) -> bool {
    let mut words = args.iter().map(String::as_str).peekable();
    let mut positional = Vec::new();
    while let Some(w) = words.next() {
        match w {
            "-h" | "--help" | "help" | "--version" | "-V" => return true,
            "--format" => {
                words.next();
            }
            w if w.starts_with("--format=") => {}
            w if positional.len() < 2 && !w.starts_with('-') => positional.push(w),
            _ => {}
        }
    }
    match positional.as_slice() {
        [group, sub] => FERRULE_READS
            .iter()
            .any(|(g, subs)| g == group && subs.contains(sub)),
        _ => false,
    }
}

/// Git's global options that change nothing about what runs.
const GIT_GLOBAL: &[&str] = &[
    "--no-pager",
    "-P",
    "--no-optional-locks",
    "--literal-pathspecs",
];

/// The flags of a program's arguments before any `--`.
fn flags(args: &[String]) -> impl Iterator<Item = &str> {
    args.iter()
        .map(String::as_str)
        .take_while(|a| *a != "--")
        .filter(|a| a.starts_with('-') && a.len() > 1)
}

/// Whether a short option `c` sits in a flag, alone or grouped (`-no`).
fn short(args: &[String], c: char) -> bool {
    flags(args).any(|a| !a.starts_with("--") && a.contains(c))
}

/// Whether a flag is `--name` or an abbreviation of it at least `min`
/// characters long (`--ou` for `--output`), with or without `=value`.
fn long(args: &[String], name: &str, min: usize) -> bool {
    flags(args).any(|a| {
        let a = a.split('=').next().unwrap_or(a);
        a.len() >= min && name.starts_with(a)
    })
}

/// A sed script's commands that write or run: `w`, `W`, `e`, and the `w`
/// and `e` flags of `s`.
fn sed_writes(script: &str) -> bool {
    let c: Vec<char> = script.chars().collect();
    let mut i = 0;
    // Past the delimited field starting after `i`.
    let field = |i: &mut usize, d: char| {
        while *i < c.len() && c[*i] != d {
            *i += if c[*i] == '\\' { 2 } else { 1 };
        }
        *i += 1;
    };
    let to_eol = |i: &mut usize| {
        while *i < c.len() && c[*i] != '\n' {
            *i += 1;
        }
    };
    while i < c.len() {
        match c[i] {
            ' ' | '\t' | '\n' | ';' | '{' | '}' | '!' | ',' | '~' | '+' | '$' => i += 1,
            d if d.is_ascii_digit() => i += 1,
            '/' => {
                i += 1;
                field(&mut i, '/');
            }
            '\\' => {
                let Some(&d) = c.get(i + 1) else { return true };
                i += 2;
                field(&mut i, d);
            }
            'w' | 'W' | 'e' => return true,
            's' | 'y' => {
                let Some(&d) = c.get(i + 1) else { return true };
                let sub = c[i] == 's';
                i += 2;
                field(&mut i, d);
                field(&mut i, d);
                while sub && i < c.len() && !matches!(c[i], ';' | '\n' | '}' | ' ' | '#') {
                    if matches!(c[i], 'w' | 'e') {
                        return true;
                    }
                    i += 1;
                }
            }
            '#' | 'a' | 'i' | 'c' | 'r' | 'R' => to_eol(&mut i),
            ':' | 'b' | 't' | 'T' => {
                while i < c.len() && !matches!(c[i], ';' | '\n') {
                    i += 1;
                }
            }
            _ => i += 1,
        }
    }
    false
}

/// `sed -n` with a script that only prints.
fn sed_reads(args: &[String]) -> bool {
    let quiet = short(args, 'n') || long(args, "--quiet", 3) || long(args, "--silent", 3);
    if !quiet
        || short(args, 'i')
        || short(args, 'f')
        || long(args, "--in-place", 4)
        || long(args, "--file", 3)
    {
        return false;
    }
    let mut scripts: Vec<&str> = Vec::new();
    let mut positional = None;
    let mut i = 0;
    while let Some(a) = args.get(i) {
        if a == "--" {
            positional = positional.or(args.get(i + 1));
            break;
        } else if a == "--expression"
            || (!a.starts_with("--") && a.starts_with('-') && a.ends_with('e'))
        {
            scripts.extend(args.get(i + 1).map(String::as_str));
            i += 1;
        } else if let Some(s) = a.strip_prefix("--expression=") {
            scripts.push(s);
        } else if !a.starts_with('-') {
            positional = positional.or(Some(a));
        }
        i += 1;
    }
    if scripts.is_empty() {
        scripts.extend(positional.map(String::as_str));
    }
    !scripts.iter().any(|s| sed_writes(s))
}

/// An awk program that writes a file, runs a command or loads code.
fn awk_reads(args: &[String]) -> bool {
    if short(args, 'f')
        || short(args, 'i')
        || short(args, 'l')
        || long(args, "--file", 3)
        || long(args, "--include", 3)
        || long(args, "--load", 3)
    {
        return false;
    }
    !args.iter().any(|a| {
        let squeezed: String = a.chars().filter(|c| !c.is_whitespace()).collect();
        squeezed.contains("system(")
            || squeezed.contains("|getline")
            || squeezed.contains("|&")
            || squeezed.contains("@include")
            || squeezed.contains("@load")
            || a.match_indices("print").any(|(at, _)| {
                let rest = &a[at + 5..];
                let rest = rest.split([';', '}', '\n']).next().unwrap_or(rest);
                rest.contains(['>', '|'])
            })
    })
}

/// `uniq [INPUT [OUTPUT]]`: an output file is the second operand.
fn uniq_reads(args: &[String]) -> bool {
    let mut operands = 0;
    let mut skip = false;
    for a in args {
        if skip {
            skip = false;
        } else if matches!(a.as_str(), "-f" | "-s" | "-w") {
            skip = true;
        } else if !a.starts_with('-') || a == "-" {
            operands += 1;
        }
    }
    operands < 2
}

/// A git call that only reads: a listed subcommand, no `-c` or other
/// global option that could run a command, no option writing a file.
fn git_reads(args: &[String]) -> bool {
    let mut i = 0;
    while let Some(w) = args.get(i) {
        if w == "-C" {
            i += 2;
        } else if GIT_GLOBAL.contains(&w.as_str()) {
            i += 1;
        } else {
            break;
        }
    }
    let Some(sub) = args.get(i) else { return false };
    let rest = &args[i + 1..];
    match sub.as_str() {
        "log" | "show" | "diff" => !long(rest, "--output", 4) && !long(rest, "--ext-diff", 5),
        "grep" => {
            !long(rest, "--open-files-in-pager", 3) && !flags(rest).any(|a| a.starts_with("-O"))
        }
        "status" | "blame" | "rev-parse" | "ls-files" | "ls-tree" | "cat-file" | "merge-base"
        | "describe" | "shortlog" => true,
        "branch" | "tag" => readonly_git(sub, rest),
        _ => false,
    }
}

/// A cargo call that builds or inspects: no `--fix`, no `--config`.
fn cargo_reads(args: &[String]) -> bool {
    matches!(
        args.first().map(String::as_str),
        Some("test" | "check" | "clippy" | "tree" | "metadata")
    ) && !has(args, &["--fix", "--config"])
}

/// Whether the program is one of the read-only programs, in a read-only
/// form.
fn allowed_program(name: &str, args: &[String]) -> bool {
    match name {
        n if PLAIN.contains(&n) => true,
        "rg" => !has(args, &["--pre", "--hostname-bin"]),
        "file" => !short(args, 'C') && !long(args, "--compile", 3),
        "tree" => !short(args, 'o'),
        "find" => !has(
            args,
            &[
                "-delete", "-exec", "-execdir", "-ok", "-okdir", "-fprint", "-fprint0", "-fprintf",
                "-fls",
            ],
        ),
        "sed" => sed_reads(args),
        "awk" | "gawk" | "mawk" => awk_reads(args),
        "sort" => {
            !short(args, 'o') && !long(args, "--output", 3) && !long(args, "--compress-program", 4)
        }
        "uniq" => uniq_reads(args),
        "ferrule" => ferrule_reads(args),
        "git" => git_reads(args),
        "cargo" => cargo_reads(args),
        _ => false,
    }
}

fn writable(cwd: &Path, target: &str, append: bool) -> bool {
    if target == "/dev/null" {
        return true;
    }
    let path = crate::hook::guard::resolve(cwd, target);
    let path = path.to_string_lossy();
    is_artifact(&path, "review.verdict.md") || (append && is_artifact(&path, "review.md"))
}

/// The reviewer's guard: only the allowlisted read-only programs, and no
/// write except appending to the run's `review.md` and rewriting its
/// `review.verdict.md`. `mutating` adds word prefixes to refuse
/// (`cargo add`); `allow` adds word prefixes to let through (`make check`).
#[must_use]
pub fn readonly(
    command: &str,
    cwd: &Path,
    mutating: &[String],
    allow: &[String],
) -> Option<String> {
    let mut cwd = cwd.to_path_buf();
    for c in shell::parse(command) {
        let refuse = |why: &str| {
            Some(format!(
                "fe-reviewer is read-only (hook-enforced): '{}' {why}. The only sanctioned writes are an append ('>>' or 'tee -a') onto .ferrule/runs/<slug>/review.md and a rewrite ('>' or 'tee') of .ferrule/runs/<slug>/review.verdict.md. Never run git checkout/stash/reset or anything that touches the worktree; record what you needed to probe as a finding for the implementer instead.",
                c.words.join(" ")
            ))
        };
        if let Some(o) = c.opaque {
            return Some(opaque_reason(o).to_owned());
        }
        let program = shell::program(&c.words);
        if let Some(("cd", args)) = program {
            if let Some(dir) = cd_target(&cwd, args) {
                cwd = dir;
            }
            continue;
        }
        if let Some((name, args)) = program {
            if shell::wrapped(&c.words) {
                return refuse(
                    "runs behind a wrapper or an assignment (env, timeout, nice, A=b …) the reviewer's guard does not follow; run the program itself",
                );
            }
            let ok = match name {
                "tee" => {
                    let (append, targets) = tee_targets(args);
                    targets.iter().all(|t| writable(&cwd, t, append))
                }
                _ => {
                    (allowed_program(name, args) || has_prefix(name, args, allow))
                        && !has_prefix(name, args, mutating)
                }
            };
            if !ok {
                return refuse(
                    "is not a read-only command on the reviewer's allowlist (ls cat head tail wc grep rg find sed -n awk diff cmp file stat tree sort uniq cut tr nl jq, git log|show|diff|status|blame|grep|rev-parse|ls-files|ls-tree|cat-file|merge-base|describe|shortlog, cargo test|check|clippy|tree|metadata, ferrule; `[hook.readonly] allow` in ferrule.toml adds more)",
                );
            }
        }
        let redirects_ok = c.redirects.iter().all(|r| match r.kind {
            RedirectKind::Append => writable(&cwd, &r.target, true),
            RedirectKind::Write => writable(&cwd, &r.target, false),
            _ => true,
        });
        if !redirects_ok {
            return refuse("writes a file");
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(cmd: &str) -> Option<String> {
        readonly(cmd, Path::new("/nonexistent-ferrule-guard"), &[], &[])
    }

    #[test]
    fn ut_bypasses_of_a_denylist_are_refused() {
        for cmd in [
            "timeout 5 rm -rf x",
            "timeout 5 cat f",
            "env A=b cat f",
            "A=b cat f",
            "nice cat f",
            "patch -p1 < x.diff",
            "git bisect start",
            "git sparse-checkout set x",
            "git diff --output=src/main.rs",
            "git log -p --output=x",
            "git show --ext-diff",
            "git -c alias.x='!rm -rf .' x",
            "git -c core.fsmonitor=rm status",
            "git --git-dir=x log",
            "git grep -O sh x",
            "git branch new",
            "git tag v1",
            "sed -n 'w out' f",
            "sed 's/a/b/w out' f",
            "sed -n 's/a/b/e' f",
            "sed -n -e p -e 'w out' f",
            "sed -n --expression='1w out' f",
            "sed -n -f s.sed f",
            "sed 's/a/b/' f",
            "sed -ni p f",
            "sed -i s/a/b/ f",
            "awk 'BEGIN{print 1 > \"f\"}'",
            "awk '{ if (1) print $1 > \"f\" }' x",
            "awk 'BEGIN { system(\"rm x\") }'",
            "awk 'BEGIN { \"date\" | getline d }'",
            "awk -f prog.awk x",
            "tar xf a.tar",
            "unzip a.zip",
            "curl -o x http://y",
            "rsync a b",
            "gzip f",
            "sort -o f f",
            "sort --output=f f",
            "sort -no f f",
            "uniq in out",
            "rustfmt src/x.rs",
            "make",
            "npm install",
            "cargo run",
            "cargo fmt",
            "cargo clippy --fix",
            "cargo test --config x=y",
            "uv run x",
            "bash <<< \"rm -rf x\"",
            "bash -c ls",
            "sh script.sh",
            "eval ls",
            "$(echo rm) -rf x",
            "$'rm' -rf x",
            "find . -exec rm {} ;",
            "find . -delete",
            "find . -fprint out",
            "rg --pre sh x",
            "tree -o out",
            "file -C",
            "echo x > f",
            "echo x >> f",
            "cat a | tee out",
            "ls $(rm x)",
            "python3 -c 1",
            "gh pr view 3",
            "ferrule pr push s",
            "ferrule stage commit s.s1 -m x",
            "ferrule board move s.s1 done",
            "ferrule board log s.s1 note -- x",
            "ferrule setup uninstall --purge",
            "ferrule prompt show f",
            "ferrule",
            "gawk 'BEGIN { \"date\" |& getline d }'",
            "gawk '@include \"x.awk\"' f",
            "sort --compress-program=sh f",
            "sort --compress=sh f",
        ] {
            assert!(check(cmd).is_some(), "{cmd}");
        }
    }

    #[test]
    fn ut_reviewer_commands_pass() {
        for cmd in [
            "git diff main...HEAD -- src",
            "git -C sub log --oneline -5",
            "git --no-pager show HEAD",
            "git grep -n foo main -- docs",
            "git branch",
            "git branch --list 'x*'",
            "git tag -l",
            "rg foo src",
            "cargo test -q",
            "cargo test 2>&1 | tail -5",
            "cargo clippy --all-targets -q",
            "ferrule run diff s --specs",
            "ferrule --format md board show s.s1",
            "ferrule file section 'S1' f",
            "ferrule spec refs X-R-001",
            "ferrule pr push --help",
            "sed -n 1,20p f",
            "sed -n '/web/p;$p' f",
            "sed -n -e 's/a/web/p' f",
            "sed --quiet 2p f",
            "cat a | head",
            "awk '$2 > 5 { print $1 }' f",
            "sort -u f | uniq -c | wc -l",
            "uniq -f 1 f",
            "grep -rn '=>' src > /dev/null",
            "find . -name '*.rs' -newer f",
            "for f in a b; do cat $f; done",
            "if [ -f x ]; then cat x; fi",
            "echo x >> .ferrule/runs/demo/review.md",
            "cat > .ferrule/runs/demo/review.verdict.md <<EOF\nx\nEOF",
            "x=$(git log -1); echo $x",
        ] {
            assert_eq!(check(cmd), None, "{cmd}");
        }
    }

    #[test]
    fn ut_allow_extends_and_mutating_still_refuses() {
        let allow = vec!["make check".to_owned()];
        let at = |c: &str, mutating: &[String]| {
            readonly(c, Path::new("/nonexistent-ferrule-guard"), mutating, &allow)
        };
        assert_eq!(at("make check", &[]), None);
        assert_eq!(at("make check ARGS=x", &[]), None);
        assert!(at("make", &[]).is_some());
        assert!(at("make install", &[]).is_some());
        assert!(at("make check", &["make check".to_owned()]).is_some());
    }
}
