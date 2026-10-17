//! Guards on the shell commands an agent runs: each returns the reason a
//! command is refused, or `None` to let it through.

use std::path::{Component, Path, PathBuf};
use std::process::Command as Process;

use crate::shell::{self, Command, RedirectKind};

/// Board state directories a card lives in.
const CARD_STATES: &[&str] = &["open", "inprogress", "inreview", "done"];
const PROTECTED_BRANCH: &str = "main";
/// A plain `cat` of a longer file is refused.
const LARGE_LINES: usize = 80;

fn components(path: &str) -> Vec<&str> {
    Path::new(path)
        .components()
        .filter_map(|c| match c {
            Component::Normal(s) => s.to_str(),
            _ => None,
        })
        .collect()
}

/// `…/tasks/<state>/<card>.md`.
fn is_card(path: &str) -> bool {
    let c = components(path);
    matches!(c.as_slice(), [.., "tasks", state, card]
        if CARD_STATES.contains(state) && is_markdown(card))
}

fn is_markdown(path: &str) -> bool {
    Path::new(path)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("md"))
}

/// `…/.claude/tasks/artifacts/<slug>/<file>`.
fn is_artifact(path: &str, file: &str) -> bool {
    let c = components(path);
    matches!(c.as_slice(), [.., ".claude", "tasks", "artifacts", _, f] if *f == file)
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

fn branch(dir: &Path) -> Option<String> {
    let out = Process::new("git")
        .arg("-C")
        .arg(dir)
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
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

/// The commands every session may not run raw: card edits that bypass the
/// card tools, output that floods the context, the issue and PR views that
/// crash on GitHub's Projects-Classic bug, and commits or pushes to `main`.
#[must_use]
pub fn shell(command: &str, cwd: &Path) -> Option<String> {
    let mut cwd = cwd.to_path_buf();
    for c in shell::parse(command) {
        let Some((name, args)) = shell::program(&c.words) else {
            if let Some(reason) = card_redirect(&c) {
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
        let reason = card_redirect(&c)
            .or_else(|| card_edit(name, args))
            .or_else(|| flood(&c, name, args, &cwd))
            .or_else(|| github_view(name, args))
            .or_else(|| protected_branch(name, args, &cwd));
        if reason.is_some() {
            return reason;
        }
    }
    None
}

fn card_redirect(c: &Command) -> Option<String> {
    c.redirects
        .iter()
        .any(|r| matches!(r.kind, RedirectKind::Write | RedirectKind::Append) && is_card(&r.target))
        .then(|| {
            "Raw write onto a task card bypasses its validation. Use: sh .claude/scripts/card-log.sh <card-id> <event> [k=v …] [-- <text>] (vocabulary in the script).".to_owned()
        })
}

fn card_edit(name: &str, args: &[String]) -> Option<String> {
    match name {
        "tee" if tee_targets(args).1.iter().any(|t| is_card(t)) => Some(
            "Raw write onto a task card bypasses its validation. Use: sh .claude/scripts/card-log.sh <card-id> <event> [k=v …] [-- <text>] (vocabulary in the script).".to_owned(),
        ),
        "mv" if args.iter().any(|a| is_card(a)) => Some(
            "Raw mv of a task card skips the transition rules. Use: sh .claude/scripts/card-move.sh <card-id> <open|inprogress|inreview|done>.".to_owned(),
        ),
        _ => None,
    }
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

fn github_view(name: &str, args: &[String]) -> Option<String> {
    if name != "gh" {
        return None;
    }
    let words: Vec<&str> = args
        .iter()
        .map(String::as_str)
        .filter(|a| !a.starts_with('-'))
        .collect();
    match words.as_slice() {
        ["issue", "view", ..] => Some(
            "Raw 'gh issue view' crashes on GitHub's Projects-Classic API bug. Use: bash .claude/scripts/issue-view.sh <number>".to_owned(),
        ),
        ["pr", "view", ..] => Some(
            "Raw 'gh pr view' crashes on GitHub's Projects-Classic API bug ('repository.pullRequest.projectCards'). Use: bash .claude/scripts/pr-view.sh <number>".to_owned(),
        ),
        _ => None,
    }
}

fn protected_branch(name: &str, args: &[String], cwd: &Path) -> Option<String> {
    if name != "git" {
        return None;
    }
    let g = shell::git(args)?;
    let dir = g.dir.map_or_else(|| cwd.to_path_buf(), |d| resolve(cwd, d));
    let on_main = || branch(&dir).as_deref() == Some(PROTECTED_BRANCH);
    let worktree = format!(
        "Create/use a git worktree first: git worktree add .claude/worktrees/<slug> -b <type>/<slug> {PROTECTED_BRANCH}"
    );
    match g.sub {
        "commit" if !g.args.iter().any(|a| a == "--dry-run") && on_main() => Some(format!(
            "'git commit' while the checkout in {} is on '{PROTECTED_BRANCH}': branch off {PROTECTED_BRANCH}, never commit to it. {worktree}",
            dir.display()
        )),
        "push" => {
            let targets_main = g.args.iter().any(|a| {
                !a.starts_with('-')
                    && (a == PROTECTED_BRANCH
                        || a.rsplit_once(':')
                            .is_some_and(|(_, dst)| dst == PROTECTED_BRANCH))
            });
            if targets_main {
                Some(format!(
                    "'git push' targeting '{PROTECTED_BRANCH}' directly: merge to {PROTECTED_BRANCH} by squash-merged PR only. Push the feature branch and open a PR instead."
                ))
            } else if on_main() {
                Some(format!(
                    "Bare 'git push' while the checkout in {} is on '{PROTECTED_BRANCH}' would push straight to it. {worktree}",
                    dir.display()
                ))
            } else {
                None
            }
        }
        _ => None,
    }
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
/// rewriting its `review.verdict.md`.
#[must_use]
pub fn readonly(command: &str, cwd: &Path) -> Option<String> {
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
            _ => readonly_program(name, args),
        });
        let redirects_ok = c.redirects.iter().all(|r| match r.kind {
            RedirectKind::Append => writable(&cwd, &r.target, true),
            RedirectKind::Write => writable(&cwd, &r.target, false),
            _ => true,
        });
        if !program_ok || !redirects_ok {
            return Some(format!(
                "spec-reviewer is read-only (hook-enforced): '{}' would alter the repo, the working tree, or the filesystem. The only sanctioned writes are an append ('>>' or 'tee -a') onto .claude/tasks/artifacts/<slug>/review.md and a rewrite ('>' or 'tee') of .claude/tasks/artifacts/<slug>/review.verdict.md. Never run git checkout/stash/reset or anything that touches the worktree; record what you needed to probe as a finding for the implementer instead.",
                c.words.join(" ")
            ));
        }
    }
    None
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
    fn ut_card_paths() {
        assert!(is_card(".claude/tasks/open/demo.s2.md"));
        assert!(is_card("/x/.claude/tasks/done/demo.md"));
        assert!(!is_card("/tmp/done/notes.md"));
        assert!(!is_card(".claude/tasks/artifacts/demo/review.md"));
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
