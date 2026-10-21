//! What a command does, by its program and arguments: the flag tests and
//! the read-only forms of git, sed, awk and find that both guards judge
//! commands by, and the word prefixes a project's settings name.

use crate::hook::shell;

/// The flags of a program's arguments before any `--`.
pub(super) fn flags(args: &[String]) -> impl Iterator<Item = &str> {
    args.iter()
        .map(String::as_str)
        .take_while(|a| *a != "--")
        .filter(|a| a.starts_with('-') && a.len() > 1)
}

/// Whether a short option `c` sits in a flag, alone or grouped (`-no`).
pub(super) fn short(args: &[String], c: char) -> bool {
    flags(args).any(|a| !a.starts_with("--") && a.contains(c))
}

/// Whether a flag is `--name` or an abbreviation of it at least `min`
/// characters long (`--ou` for `--output`), with or without `=value`.
pub(super) fn long(args: &[String], name: &str, min: usize) -> bool {
    flags(args).any(|a| {
        let a = a.split('=').next().unwrap_or(a);
        a.len() >= min && name.starts_with(a)
    })
}

/// Whether `args` holds any of `flags`, alone or as a `--flag=value`.
pub(super) fn has(args: &[String], flags: &[&str]) -> bool {
    args.iter().any(|a| {
        flags.iter().any(|f| {
            a == f || (f.starts_with("--") && a.strip_prefix(f).is_some_and(|r| r.starts_with('=')))
        })
    })
}

/// Whether `words` start with the words of one of `prefixes`.
pub(super) fn word_prefix(words: &[&str], prefixes: &[String]) -> bool {
    prefixes.iter().any(|p| {
        let p: Vec<&str> = p.split_whitespace().collect();
        !p.is_empty() && words.starts_with(&p)
    })
}

/// Whether the command's words start with one of `prefixes`' words.
pub(super) fn has_prefix(name: &str, args: &[String], prefixes: &[String]) -> bool {
    let words: Vec<&str> = std::iter::once(name)
        .chain(args.iter().map(String::as_str))
        .collect();
    word_prefix(&words, prefixes)
}

const GIT_WRITES: &[&str] = &[
    "checkout",
    "switch",
    "restore",
    "reset",
    "clean",
    "commit",
    "add",
    "rm",
    "mv",
    "merge",
    "rebase",
    "cherry-pick",
    "revert",
    "apply",
    "am",
    "push",
    "pull",
    "notes",
    "update-ref",
    "symbolic-ref",
    "filter-branch",
    "gc",
    "prune",
    "reflog",
    "submodule",
    "remote",
    "init",
    "clone",
];

pub(super) fn readonly_git(sub: &str, args: &[String]) -> bool {
    let positional: Vec<&str> = args
        .iter()
        .map(String::as_str)
        .filter(|a| !a.starts_with('-'))
        .collect();
    match sub {
        s if GIT_WRITES.contains(&s) => false,
        "stash" => matches!(positional.first(), Some(&("list" | "show"))),
        "worktree" => matches!(positional.first(), Some(&"list")),
        "fetch" => !has(args, &["--prune", "-p", "-P", "--prune-tags"]),
        "branch" => {
            !has(
                args,
                &[
                    "-d",
                    "-D",
                    "-m",
                    "-M",
                    "-c",
                    "-C",
                    "--delete",
                    "--move",
                    "--copy",
                    "-f",
                    "--force",
                    "-u",
                    "--set-upstream-to",
                    "--unset-upstream",
                    "--edit-description",
                ],
            ) && (positional.is_empty()
                || has(
                    args,
                    &[
                        "-l",
                        "--list",
                        "--contains",
                        "--merged",
                        "--no-merged",
                        "--points-at",
                    ],
                ))
        }
        "tag" => positional.is_empty() || has(args, &["-l", "--list"]),
        "config" => {
            has(
                args,
                &["--get", "--get-all", "--get-regexp", "-l", "--list"],
            ) || matches!(positional.first(), Some(&("get" | "list")))
        }
        _ => true,
    }
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

/// The scripts of a sed call: its `-e`/`--expression` ones, else its
/// first operand.
fn sed_scripts(args: &[String]) -> Vec<&str> {
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
    scripts
}

/// A sed call that edits in place, reads its script from a file or has a
/// script that writes or runs.
pub(super) fn sed_edits(args: &[String]) -> bool {
    short(args, 'i')
        || short(args, 'f')
        || long(args, "--in-place", 4)
        || long(args, "--file", 3)
        || sed_scripts(args).iter().any(|s| sed_writes(s))
}

/// `sed -n` with a script that only prints.
pub(super) fn sed_reads(args: &[String]) -> bool {
    let quiet = short(args, 'n') || long(args, "--quiet", 3) || long(args, "--silent", 3);
    quiet && !sed_edits(args)
}

/// An awk program that writes a file, runs a command or loads code.
pub(super) fn awk_reads(args: &[String]) -> bool {
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

/// A find call that deletes, runs a command or writes a file.
pub(super) fn find_writes(args: &[String]) -> bool {
    has(
        args,
        &[
            "-delete", "-exec", "-execdir", "-ok", "-okdir", "-fprint", "-fprint0", "-fprintf",
            "-fls",
        ],
    )
}

/// Whether git, sed, awk or find runs in a form that only reads; any
/// other program is not judged here.
pub(super) fn reads(name: &str, args: &[String]) -> bool {
    match name {
        "git" => shell::git(args).is_none_or(|g| readonly_git(g.sub, g.args)),
        "sed" => !sed_edits(args),
        "awk" | "gawk" | "mawk" => awk_reads(args),
        "find" => !find_writes(args),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ut_awk_reads_are_not_writes() {
        let a = |s: &str| vec![s.to_owned()];
        assert!(awk_reads(&a("$2 > 5 { print $1 }")));
        assert!(!awk_reads(&a("{ print $1 > \"out\" }")));
        assert!(!awk_reads(&a("{ system(\"rm x\") }")));
        assert!(!awk_reads(&a("{ print | \"sh\" }")));
    }

    #[test]
    fn ut_sed_that_writes_is_no_reader() {
        let a = |v: &[&str]| v.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        assert!(reads("sed", &a(&["s/a/b/", "f"])));
        assert!(!reads("sed", &a(&["-i", "s/a/b/", "f"])));
        assert!(!reads("sed", &a(&["s/a/b/w .ferrule/x", "f"])));
        assert!(!reads("sed", &a(&["-n", "1e rm x", "f"])));
        assert!(!reads("find", &a(&[".", "-delete"])));
        assert!(reads("git", &a(&["log"])));
        assert!(!reads("git", &a(&["reset", "--hard"])));
    }
}
