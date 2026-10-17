//! Guards on the shell commands an agent runs: each returns the reason a
//! command is refused, or `None` to let it through.

use std::path::{Component, Path, PathBuf};

use crate::shell::{self, Command, RedirectKind};

/// A plain `cat` of a longer file is refused.
const LARGE_LINES: usize = 80;

/// What a session may not run raw, and what to run instead.
#[derive(Debug, Clone, Copy)]
pub struct Replaced {
    /// The program and its leading words (`gh pr create`).
    pub raw: &'static str,
    pub instead: &'static str,
    pub why: &'static str,
}

/// Every raw command ferrule runs itself. `ferrule install` writes the
/// `permissions.deny` backstop from this list, so the guard and the rules
/// cannot drift apart.
pub const REPLACED: &[Replaced] = &[
    Replaced {
        raw: "git commit",
        instead: "ferrule card commit <card> -m <msg> (outside a run: ferrule card commit --no-run -m <msg>)",
        why: "it checks the stage card and the branch, strips attribution trailers and logs commit=",
    },
    Replaced {
        raw: "git push",
        instead: "ferrule pr push <slug> (outside a run: ferrule pr push --no-run)",
        why: "it runs the sha check and the gauntlet first and refreshes the draft PR",
    },
    Replaced {
        raw: "git worktree add",
        instead: "ferrule run approve gate2 <slug>",
        why: "a run's worktree lives and dies with the run",
    },
    Replaced {
        raw: "git worktree remove",
        instead: "ferrule run cleanup <slug>",
        why: "a run's worktree lives and dies with the run",
    },
    Replaced {
        raw: "gh issue create",
        instead: "ferrule issue file <slug> [--slice <n>|--split|--follow-up <k>]",
        why: "title and body come from the run's files; an open issue with the same title is adopted",
    },
    Replaced {
        raw: "gh issue edit",
        instead: "ferrule issue file <slug> --reuse <n>",
        why: "title and body come from the run's files",
    },
    Replaced {
        raw: "gh issue comment",
        instead: "ferrule issue file <slug> --comment",
        why: "the comment is the run's issue-comment.md",
    },
    Replaced {
        raw: "gh pr create",
        instead: "ferrule pr open <slug>, or the run's first ferrule pr push",
        why: "the body comes from the run state",
    },
    Replaced {
        raw: "gh pr edit",
        instead: "ferrule card move (re-renders the draft) or ferrule pr publish <slug>",
        why: "the body comes from the run state",
    },
    Replaced {
        raw: "gh pr ready",
        instead: "ferrule pr publish <slug>",
        why: "publishing checks threads and ID collisions first",
    },
    Replaced {
        raw: "gh pr comment",
        instead: "ferrule pr feedback reply <pr> <file> [<stage> <sha>]",
        why: "replies follow the threads file",
    },
    Replaced {
        raw: "gh issue view",
        instead: "ferrule issue view <n>",
        why: "the raw view crashes on GitHub's Projects-Classic API bug",
    },
    Replaced {
        raw: "gh pr view",
        instead: "ferrule pr view <n>",
        why: "the raw view crashes on GitHub's Projects-Classic API bug",
    },
    Replaced {
        raw: "gh run view",
        instead: "ferrule pr failed <branch>",
        why: "it prints only the failed job's errors",
    },
];

/// Files only ferrule writes, under `.ferrule/`.
const OWNED: &[&str] = &["state.db", "ferrule.lock"];

/// The guard's project settings.
#[derive(Debug, Clone, Copy)]
pub struct Policy<'a> {
    /// `[guard] allow`: raw commands re-opened.
    pub allow: &'a [String],
}

fn components(path: &str) -> Vec<&str> {
    Path::new(path)
        .components()
        .filter_map(|c| match c {
            Component::Normal(s) => s.to_str(),
            _ => None,
        })
        .collect()
}

/// `….ferrule/state.db` (and its journal files) or `….ferrule/ferrule.lock`.
fn is_owned(path: &str) -> bool {
    let c = components(path);
    matches!(c.as_slice(), [.., ".ferrule", f]
        if OWNED.iter().any(|o| f.strip_prefix(o).is_some_and(|r| r.is_empty() || r.starts_with('-'))))
}

fn is_markdown(path: &str) -> bool {
    Path::new(path)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("md"))
}

/// `…/.ferrule/runs/<slug>/<file>`.
fn is_artifact(path: &str, file: &str) -> bool {
    let c = components(path);
    matches!(c.as_slice(), [.., ".ferrule", "runs", _, f] if *f == file)
}

fn resolve(cwd: &Path, path: &str) -> PathBuf {
    match path.strip_prefix("~/") {
        Some(rest) => std::env::home_dir().unwrap_or_default().join(rest),
        None => cwd.join(path),
    }
}

/// Words after `tee`'s options, and whether `-a`/`--append` was given.
fn tee_targets(args: &[String]) -> (bool, Vec<&str>) {
    let mut append = false;
    let mut targets = Vec::new();
    let mut options = true;
    for a in args {
        match a.as_str() {
            "--" if options => options = false,
            "--append" if options => append = true,
            o if options && o.starts_with('-') && o.len() > 1 => {
                append |= !o.starts_with("--") && o.contains('a');
            }
            t => targets.push(t),
        }
    }
    (append, targets)
}

/// The directory `cd <args>` moves to, if it exists.
fn cd_target(cwd: &Path, args: &[String]) -> Option<PathBuf> {
    let target = args
        .iter()
        .skip_while(|a| a.starts_with('-') && a.as_str() != "--")
        .find(|a| a.as_str() != "--")?;
    let dir = resolve(cwd, target);
    dir.is_dir().then_some(dir)
}

/// The commands every session may not run raw: writes to the files only
/// ferrule writes, output that floods the context, and the git and `gh`
/// calls ferrule runs itself ([`REPLACED`]) unless the project re-opened
/// them.
#[must_use]
pub fn shell(command: &str, cwd: &Path, policy: Policy) -> Option<String> {
    let mut cwd = cwd.to_path_buf();
    for c in shell::parse(command) {
        let Some((name, args)) = shell::program(&c.words) else {
            if let Some(reason) = owned_redirect(&c) {
                return Some(reason);
            }
            continue;
        };
        if name == "cd" {
            if let Some(dir) = cd_target(&cwd, args) {
                cwd = dir;
            }
            continue;
        }
        let reason = owned_redirect(&c)
            .or_else(|| owned_edit(name, args))
            .or_else(|| flood(&c, name, args, &cwd))
            .or_else(|| replaced(name, args, policy));
        if reason.is_some() {
            return reason;
        }
    }
    None
}

/// Output that would land whole in the context: only when nothing captures
/// it.
fn flood(c: &Command, name: &str, args: &[String], cwd: &Path) -> Option<String> {
    if c.output_captured() {
        return None;
    }
    match name {
        "cat" => args.iter().filter(|a| !a.starts_with('-')).find_map(|a| {
            let path = resolve(cwd, a);
            if !path.is_file() {
                return None;
            }
            if is_markdown(a) {
                return Some(format!(
                    "Whole-file cat of a .md file. Run: ferrule spec sections {a} to see headings, then ferrule spec section '<heading>' {a} for just what's needed. Genuinely need the whole document? Use the Read tool instead."
                ));
            }
            let lines = std::fs::read_to_string(&path).map_or(0, |t| t.lines().count());
            (lines > LARGE_LINES).then(|| format!(
                "Whole-file cat of a {lines}-line file. Use the Read tool (with offset/limit if only part is needed) or sed -n 'START,ENDp' {a}."
            ))
        }),
        "git" => {
            let g = shell::git(args)?;
            let narrowed = g.args.iter().any(|a| {
                matches!(
                    a.as_str(),
                    "--stat" | "--shortstat" | "--numstat" | "--name-only" | "--name-status" | "--quiet" | "--"
                ) || a.starts_with("--stat=")
                    || (g.sub == "show" && !a.starts_with('-') && a.contains(':'))
            });
            (matches!(g.sub, "diff" | "show") && !narrowed).then(|| {
                "Unfiltered 'git show'/'git diff'. Add --stat first, scope with a pathspec ('-- <path>'), or pipe through head/grep.".to_owned()
            })
        }
        "find" => {
            let typed = args.iter().any(|a| a == "-type");
            let narrowed = args
                .iter()
                .any(|a| matches!(a.as_str(), "-name" | "-iname" | "-path" | "-ipath" | "-regex" | "-iregex"));
            (typed && !narrowed).then(|| {
                "Unfiltered 'find -type'. Narrow with -name/-path/-iname/-regex, or pipe through head/grep.".to_owned()
            })
        }
        _ => None,
    }
}

const OWNED_REASON: &str = "Only ferrule writes .ferrule/state.db and .ferrule/ferrule.lock. Use ferrule card log|move|set for the board, ferrule card show|board to read it, ferrule install for the lock.";

fn owned_redirect(c: &Command) -> Option<String> {
    c.redirects
        .iter()
        .any(|r| {
            matches!(r.kind, RedirectKind::Write | RedirectKind::Append) && is_owned(&r.target)
        })
        .then(|| OWNED_REASON.to_owned())
}

fn owned_edit(name: &str, args: &[String]) -> Option<String> {
    let hit = match name {
        "tee" => tee_targets(args).1.iter().any(|t| is_owned(t)),
        "mv" | "cp" | "rm" | "sqlite3" | "truncate" | "dd" => args
            .iter()
            .any(|a| is_owned(a) || a.strip_prefix("of=").is_some_and(is_owned)),
        _ => false,
    };
    hit.then(|| OWNED_REASON.to_owned())
}

/// The command's words that name what it does: the program, then its
/// non-option words (`git -C x commit -m y` → `git commit y`).
fn verb_words<'a>(name: &'a str, args: &'a [String]) -> Option<Vec<&'a str>> {
    let rest: Vec<&str> = match name {
        "git" => {
            let g = shell::git(args)?;
            std::iter::once(g.sub)
                .chain(
                    g.args
                        .iter()
                        .map(String::as_str)
                        .filter(|a| !a.starts_with('-')),
                )
                .collect()
        }
        "gh" => args
            .iter()
            .map(String::as_str)
            .filter(|a| !a.starts_with('-'))
            .collect(),
        _ => return None,
    };
    Some(std::iter::once(name).chain(rest).collect())
}

fn replaced(name: &str, args: &[String], policy: Policy) -> Option<String> {
    if has_prefix(name, args, policy.allow) {
        return None;
    }
    let words = verb_words(name, args)?;
    if words.starts_with(&["git", "commit"]) && args.iter().any(|a| a == "--dry-run") {
        return None;
    }
    if words.starts_with(&["gh", "api"]) {
        let writes = has(args, &["-f", "-F", "--field", "--raw-field", "--input"])
            || args.windows(2).any(|w| {
                matches!(w[0].as_str(), "-X" | "--method") && !w[1].eq_ignore_ascii_case("GET")
            })
            || args.iter().any(|a| {
                a.strip_prefix("--method=")
                    .is_some_and(|m| !m.eq_ignore_ascii_case("GET"))
            });
        return writes.then(|| {
            "A writing 'gh api' call is ferrule's to run here. Use: ferrule pr feedback reply <pr> <file> [<stage> <sha>] for review threads, ferrule issue file / ferrule pr publish for the rest. Re-open it with [guard] allow = [\"gh api\"] in .ferrule/ferrule.toml if the project needs it.".to_owned()
        });
    }
    let r = REPLACED.iter().find(|r| {
        let raw: Vec<&str> = r.raw.split(' ').collect();
        words.starts_with(&raw)
    })?;
    Some(format!(
        "'{}' is ferrule's to run in this project: {}. Use: {}",
        r.raw, r.why, r.instead
    ))
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

/// `gh <group> <verb>` verbs that change something on GitHub or locally.
const GH_WRITES: &[&str] = &[
    "create",
    "comment",
    "edit",
    "close",
    "reopen",
    "merge",
    "review",
    "delete",
    "ready",
    "lock",
    "unlock",
    "pin",
    "unpin",
    "transfer",
    "checkout",
    "develop",
    "clone",
    "fork",
    "sync",
    "rename",
    "archive",
    "unarchive",
    "set-default",
    "rerun",
    "cancel",
    "run",
    "download",
    "upload",
];

/// Whether `args` holds any of `flags`, alone or as a `--flag=value`.
fn has(args: &[String], flags: &[&str]) -> bool {
    args.iter().any(|a| {
        flags.iter().any(|f| {
            a == f || (f.starts_with("--") && a.strip_prefix(f).is_some_and(|r| r.starts_with('=')))
        })
    })
}

/// Whether an awk program writes a file or runs a command.
fn awk_writes(args: &[String]) -> bool {
    args.iter().any(|a| {
        a.contains("system(")
            || a.contains("| getline")
            || a.split(['{', ';', '}']).any(|stmt| {
                let stmt = stmt.trim_start();
                (stmt.starts_with("print") || stmt.starts_with("printf"))
                    && (stmt.contains('>') || stmt.contains('|'))
            })
    })
}

fn readonly_program(name: &str, args: &[String]) -> bool {
    match name {
        "rm" | "mv" | "cp" | "ln" | "mkdir" | "touch" | "chmod" | "chown" | "truncate" | "dd"
        | "install" | "rmdir" | "shred" | "mkfifo" | "mknod" | "eval" | "source" | "."
        | "python" | "python2" | "python3" | "node" | "ruby" | "perl" | "php" => false,
        "sh" | "bash" | "dash" | "zsh" | "fish" => !has(args, &["-c"]),
        "awk" | "gawk" | "mawk" => !awk_writes(args),
        "sed" => !args.iter().any(|a| {
            a.starts_with("--in-place")
                || (a.starts_with('-') && !a.starts_with("--") && a.contains('i'))
        }),
        "find" => !has(
            args,
            &[
                "-delete", "-exec", "-execdir", "-ok", "-okdir", "-fprint", "-fprint0", "-fprintf",
                "-fls",
            ],
        ),
        "git" => shell::git(args).is_none_or(|g| readonly_git(g.sub, g.args)),
        "cargo" => shell::git(args).is_none_or(|g| match g.sub {
            "fix" | "install" | "uninstall" | "add" | "remove" | "rm" | "update" | "publish"
            | "init" | "new" | "generate-lockfile" | "vendor" | "yank" | "login" | "logout" => {
                false
            }
            "fmt" => has(g.args, &["--check"]),
            "clippy" => !has(g.args, &["--fix"]),
            _ => true,
        }),
        "gh" => readonly_gh(args),
        _ => true,
    }
}

fn readonly_git(sub: &str, args: &[String]) -> bool {
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

fn readonly_gh(args: &[String]) -> bool {
    let positional: Vec<&str> = args
        .iter()
        .map(String::as_str)
        .filter(|a| !a.starts_with('-'))
        .collect();
    match positional.as_slice() {
        ["api", ..] => !has(
            args,
            &[
                "-X",
                "--method",
                "-f",
                "-F",
                "--field",
                "--raw-field",
                "--input",
            ],
        ),
        [
            "auth" | "config" | "alias" | "extension" | "secret" | "variable" | "ssh-key"
            | "gpg-key",
            ..,
        ] => false,
        [_, verb, ..] => !GH_WRITES.contains(verb),
        _ => true,
    }
}

/// The reviewer's guard: nothing that alters the repository, the working
/// tree or the filesystem, except appending to the run's `review.md` and
/// rewriting its `review.verdict.md`. `mutating` adds word prefixes to
/// refuse (`cargo add`).
#[must_use]
pub fn readonly(command: &str, cwd: &Path, mutating: &[String]) -> Option<String> {
    let mut cwd = cwd.to_path_buf();
    for c in shell::parse(command) {
        let program = shell::program(&c.words);
        if let Some(("cd", args)) = program {
            if let Some(dir) = cd_target(&cwd, args) {
                cwd = dir;
            }
            continue;
        }
        let program_ok = program.is_none_or(|(name, args)| match name {
            "tee" => {
                let (append, targets) = tee_targets(args);
                targets.iter().all(|t| writable(&cwd, t, append))
            }
            _ => readonly_program(name, args) && !has_prefix(name, args, mutating),
        });
        let redirects_ok = c.redirects.iter().all(|r| match r.kind {
            RedirectKind::Append => writable(&cwd, &r.target, true),
            RedirectKind::Write => writable(&cwd, &r.target, false),
            _ => true,
        });
        if !program_ok || !redirects_ok {
            return Some(format!(
                "spec-reviewer is read-only (hook-enforced): '{}' would alter the repo, the working tree, or the filesystem. The only sanctioned writes are an append ('>>' or 'tee -a') onto .ferrule/runs/<slug>/review.md and a rewrite ('>' or 'tee') of .ferrule/runs/<slug>/review.verdict.md. Never run git checkout/stash/reset or anything that touches the worktree; record what you needed to probe as a finding for the implementer instead.",
                c.words.join(" ")
            ));
        }
    }
    None
}

/// Whether the command's words start with one of `prefixes`' words.
fn has_prefix(name: &str, args: &[String], prefixes: &[String]) -> bool {
    let words: Vec<&str> = std::iter::once(name)
        .chain(args.iter().map(String::as_str))
        .collect();
    prefixes.iter().any(|p| {
        let p: Vec<&str> = p.split_whitespace().collect();
        !p.is_empty() && words.starts_with(&p)
    })
}

fn writable(cwd: &Path, target: &str, append: bool) -> bool {
    if target == "/dev/null" {
        return true;
    }
    let path = resolve(cwd, target);
    let path = path.to_string_lossy();
    is_artifact(&path, "review.verdict.md") || (append && is_artifact(&path, "review.md"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ut_owned_paths() {
        assert!(is_owned(".ferrule/state.db"));
        assert!(is_owned("/x/.ferrule/state.db-wal"));
        assert!(is_owned(".ferrule/ferrule.lock"));
        assert!(!is_owned(".ferrule/ferrule.toml"));
        assert!(!is_owned("/tmp/state.db"));
    }

    #[test]
    fn ut_tee_options() {
        let args: Vec<String> = ["-ia", "--", "-x"].map(String::from).to_vec();
        assert_eq!(tee_targets(&args), (true, vec!["-x"]));
    }

    #[test]
    fn ut_awk_reads_are_not_writes() {
        let a = |s: &str| vec![s.to_owned()];
        assert!(!awk_writes(&a("$2 > 5 { print $1 }")));
        assert!(awk_writes(&a("{ print $1 > \"out\" }")));
        assert!(awk_writes(&a("{ system(\"rm x\") }")));
        assert!(awk_writes(&a("{ print | \"sh\" }")));
    }
}
