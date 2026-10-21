//! Guards on the shell commands an agent runs: each returns the reason a
//! command is refused, or `None` to let it through.

use std::path::{Component, Path, PathBuf};

use crate::hook::shell::{self, Command, Opaque, RedirectKind, Script};

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

/// Why a commit-making git command is replaced.
const COMMIT_WHY: &str = "commits land through ferrule stage commit, which checks the stage card and the branch and strips attribution trailers";

/// What to run instead of a commit-making git command.
const COMMIT_INSTEAD: &str = "bring the change in without committing (git diff <rev> | git apply; add -R to undo one), then ferrule stage commit <card> -m <msg> (outside a run: ferrule stage commit --no-run -m <msg>)";

/// Every raw command ferrule runs itself. `ferrule setup install` writes the
/// `permissions.deny` backstop from this list, so the guard and the rules
/// cannot drift apart.
pub const REPLACED: &[Replaced] = &[
    Replaced {
        raw: "git commit",
        instead: "ferrule stage commit <card> -m <msg> (outside a run: ferrule stage commit --no-run -m <msg>)",
        why: "it checks the stage card and the branch, strips attribution trailers and logs commit=",
    },
    Replaced {
        raw: "git push",
        instead: "ferrule pr push <slug> (outside a run: ferrule pr push --no-run)",
        why: "it runs the sha check and the gauntlet first and refreshes the draft PR",
    },
    Replaced {
        raw: "git merge",
        instead: COMMIT_INSTEAD,
        why: COMMIT_WHY,
    },
    Replaced {
        raw: "git cherry-pick",
        instead: COMMIT_INSTEAD,
        why: COMMIT_WHY,
    },
    Replaced {
        raw: "git revert",
        instead: COMMIT_INSTEAD,
        why: COMMIT_WHY,
    },
    Replaced {
        raw: "git am",
        instead: "git apply <patch>, then ferrule stage commit <card> -m <msg> (outside a run: ferrule stage commit --no-run -m <msg>)",
        why: COMMIT_WHY,
    },
    Replaced {
        raw: "git commit-tree",
        instead: COMMIT_INSTEAD,
        why: COMMIT_WHY,
    },
    Replaced {
        raw: "git send-pack",
        instead: "ferrule pr push <slug> (outside a run: ferrule pr push --no-run)",
        why: "it pushes as git push does, without the sha check and the gauntlet",
    },
    Replaced {
        raw: "git worktree add",
        instead: "ferrule gate approve <slug> gate2",
        why: "a run's worktree lives and dies with the run",
    },
    Replaced {
        raw: "git worktree remove",
        instead: "ferrule run cleanup <slug>",
        why: "a run's worktree lives and dies with the run",
    },
    Replaced {
        raw: "gh issue create",
        instead: "ferrule issue post <slug> [--slice <n>|--split|--follow-up <k>]",
        why: "title and body come from the run's files; an open issue with the same title is adopted",
    },
    Replaced {
        raw: "gh issue edit",
        instead: "ferrule issue post <slug> --reuse <n>",
        why: "title and body come from the run's files",
    },
    Replaced {
        raw: "gh issue comment",
        instead: "ferrule issue post <slug> --comment",
        why: "the comment is the run's issue-comment.md",
    },
    Replaced {
        raw: "gh pr create",
        instead: "ferrule pr open <slug>, or the run's first ferrule pr push",
        why: "the body comes from the run state",
    },
    Replaced {
        raw: "gh pr edit",
        instead: "ferrule board move (re-renders the draft) or ferrule pr publish <slug>",
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
];

/// Files no session writes raw, each with the directory it sits in: the
/// board and the lock, which only ferrule writes, and the settings the
/// guard and the deny rules come from, which only the user edits.
const OWNED: &[(&str, &str)] = &[
    (".ferrule", "state.db"),
    (".ferrule", "ferrule.lock"),
    (".ferrule", "ferrule.toml"),
    (".claude", "settings.json"),
    (".claude", "settings.local.json"),
];

/// The guard's project settings.
#[derive(Debug, Clone, Copy)]
pub struct Policy<'a> {
    /// `[guard] allow`: raw commands re-opened.
    pub allow: &'a [String],
}

/// The normal components of `path`, a `..` dropping the one before it.
fn components(path: &str) -> Vec<&str> {
    let mut out = Vec::new();
    for c in Path::new(path).components() {
        match c {
            Component::Normal(s) => out.extend(s.to_str()),
            Component::ParentDir => {
                out.pop();
            }
            _ => {}
        }
    }
    out
}

/// Whether shell pattern `pat` (`*`, `?`, `[…]`) matches `name`, ignoring
/// ASCII case as a case-insensitive filesystem does. A leading dot is
/// matched only by a dot, as the shell does.
fn glob(pat: &str, name: &str) -> bool {
    let p: Vec<char> = pat.to_ascii_lowercase().chars().collect();
    let n: Vec<char> = name.to_ascii_lowercase().chars().collect();
    if n.first() == Some(&'.') && p.first() != Some(&'.') {
        return false;
    }
    glob_at(&p, &n)
}

fn glob_at(p: &[char], n: &[char]) -> bool {
    match p.split_first() {
        None => n.is_empty(),
        Some(('*', rest)) => (0..=n.len()).any(|i| glob_at(rest, &n[i..])),
        Some(('?', rest)) => !n.is_empty() && glob_at(rest, &n[1..]),
        Some(('[', rest)) => {
            let Some(close) = rest.iter().skip(1).position(|&c| c == ']').map(|i| i + 1) else {
                return n.first() == Some(&'[') && glob_at(rest, &n[1..]);
            };
            let (set, after) = (&rest[..close], &rest[close + 1..]);
            let (negate, set) = match set.split_first() {
                Some(('!' | '^', s)) => (true, s),
                _ => (false, set),
            };
            let Some((&c, n_rest)) = n.split_first() else {
                return false;
            };
            let mut hit = false;
            let mut i = 0;
            while i < set.len() {
                if set.get(i + 1) == Some(&'-') && i + 2 < set.len() {
                    hit |= (set[i]..=set[i + 2]).contains(&c);
                    i += 3;
                } else {
                    hit |= set[i] == c;
                    i += 1;
                }
            }
            hit != negate && glob_at(after, n_rest)
        }
        Some((&c, rest)) => n.first() == Some(&c) && glob_at(rest, &n[1..]),
    }
}

/// The first `{…}` group of `word` (not a `${…}`) holding a top-level
/// `,` or `..`: its byte range, braces included.
fn brace_group(word: &str) -> Option<(usize, usize)> {
    let b = word.as_bytes();
    (0..b.len())
        .filter(|&i| b[i] == b'{' && (i == 0 || b[i - 1] != b'$'))
        .find_map(|open| {
            let mut depth = 0;
            let mut list = false;
            for (i, &c) in b.iter().enumerate().skip(open) {
                match c {
                    b'{' => depth += 1,
                    b'}' => {
                        depth -= 1;
                        if depth == 0 {
                            return list.then_some((open, i));
                        }
                    }
                    b',' if depth == 1 => list = true,
                    b'.' if depth == 1 && b.get(i + 1) == Some(&b'.') => list = true,
                    _ => {}
                }
            }
            None
        })
}

/// The most words one word's brace expansion is checked as.
const BRACE_LIMIT: usize = 64;

/// `word` with its `{a,b}` groups expanded; a `{x..y}` sequence becomes
/// `*`.
fn braces(word: &str) -> Vec<String> {
    let Some((open, close)) = brace_group(word) else {
        return vec![word.to_owned()];
    };
    let (pre, inner, post) = (&word[..open], &word[open + 1..close], &word[close + 1..]);
    let mut alts = Vec::new();
    let mut depth = 0;
    let mut from = 0;
    for (i, c) in inner.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => depth -= 1,
            ',' if depth == 0 => {
                alts.push(&inner[from..i]);
                from = i + 1;
            }
            _ => {}
        }
    }
    alts.push(&inner[from..]);
    if alts.len() == 1 {
        alts = vec!["*"];
    }
    alts.iter()
        .flat_map(|a| braces(&format!("{pre}{a}{post}")))
        .take(BRACE_LIMIT)
        .collect()
}

fn globbed(word: &str) -> bool {
    word.contains(['*', '?', '[']) || brace_group(word).is_some()
}

/// Whether `f` names file `o` of [`OWNED`] (or a journal file of it), by
/// name or by a glob.
fn owned_file(o: &str, f: &str) -> bool {
    f.to_ascii_lowercase()
        .strip_prefix(o)
        .is_some_and(|r| r.is_empty() || r.starts_with('-'))
        || ["", "-wal", "-shm", "-journal"]
            .iter()
            .any(|s| glob(f, &format!("{o}{s}")))
}

/// `….ferrule/state.db` (and its journal files), any other of [`OWNED`]
/// under its directory, by name, glob or case variant.
fn is_owned(path: &str) -> bool {
    let c = components(path);
    matches!(c.as_slice(), [.., d, f] if OWNED.iter().any(|(od, of)| glob(d, od) && owned_file(of, f)))
}

/// `….ferrule` itself, by name, glob or case variant: removing or moving it
/// takes ferrule's files along.
fn is_ferrule_dir(path: &str) -> bool {
    components(path).last().is_some_and(|d| glob(d, ".ferrule"))
}

/// Whether `word`, run from `cwd`, could name one of [`OWNED`] or the
/// `.ferrule` directory: a path to it, a glob or brace form of one, inline
/// code mentioning it, or a glob in the file name directly under
/// `.ferrule/`. A name built from shell variables stays unseen.
fn names_owned(cwd: &Path, word: &str) -> bool {
    let in_dir = |d: &str| cwd.file_name().is_some_and(|n| n.eq_ignore_ascii_case(d));
    !word.is_empty()
        && braces(word).iter().any(|w| {
            let lower = w.to_ascii_lowercase();
            let resolved = resolve(cwd, w).to_string_lossy().into_owned();
            is_owned(w)
                || is_owned(&resolved)
                || is_ferrule_dir(w)
                || is_ferrule_dir(&resolved)
                || OWNED.iter().any(|(d, o)| {
                    lower.contains(&format!("{d}/{o}")) || (in_dir(d) && lower.contains(o))
                })
                || [".ferrule", ".claude"].iter().any(|d| {
                    lower
                        .split(&format!("{d}/"))
                        .skip(1)
                        .any(|f| !f.contains('/') && globbed(f))
                        || (in_dir(d) && !w.contains('/') && globbed(w))
                })
        })
}

fn is_markdown(path: &str) -> bool {
    Path::new(path)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("md"))
}

/// `…/.ferrule/runs/<slug>/<file>`.
pub(super) fn is_artifact(path: &str, file: &str) -> bool {
    let c = components(path);
    matches!(c.as_slice(), [.., ".ferrule", "runs", _, f] if *f == file)
}

pub(super) fn resolve(cwd: &Path, path: &str) -> PathBuf {
    match path.strip_prefix("~/") {
        Some(rest) => std::env::home_dir().unwrap_or_default().join(rest),
        None => cwd.join(path),
    }
}

/// Words after `tee`'s options, and whether `-a`/`--append` was given.
pub(super) fn tee_targets(args: &[String]) -> (bool, Vec<&str>) {
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

/// The word `cd <args>` moves to.
fn cd_word(args: &[String]) -> Option<&String> {
    args.iter()
        .skip_while(|a| a.starts_with('-') && a.as_str() != "--")
        .find(|a| a.as_str() != "--")
}

/// The directory `cd <args>` moves to, if it exists; a `.ferrule` the
/// guard cannot see (`cd "$(…)/.ferrule"`) is taken as moved to.
pub(super) fn cd_target(cwd: &Path, args: &[String]) -> Option<PathBuf> {
    let target = cd_word(args)?;
    let dir = resolve(cwd, target);
    (dir.is_dir() || is_ferrule_dir(target)).then_some(dir)
}

/// The commands every session may not run raw: writes to the files only
/// ferrule writes, output that floods the context, the git and `gh` calls
/// ferrule runs itself ([`REPLACED`]) unless the project re-opened them,
/// wherever they sit (behind a wrapper, in `sh -c` or `eval`, under
/// `find -exec`), and what the guard cannot follow.
#[must_use]
pub fn shell(command: &str, cwd: &Path, policy: Policy) -> Option<String> {
    let mut cwd = cwd.to_path_buf();
    guard_list(&shell::parse(command), &mut cwd, policy, 0)
}

/// Where the guard takes a session to stand after `cd -` or `popd`.
const UNKNOWN_CWD: &str = "/.ferrule";

/// How deep `sh -c` and `eval` strings are followed; deeper is refused.
const MAX_DEPTH: usize = 8;

const DEEP_REASON: &str = "Shell code nested more than 8 levels deep (sh -c, eval) is more than the guard follows. Run the inner commands directly.";

const FED_REASON: &str = "A script piped, heredoc'd or here-string'd into a shell, source or eval (or read from <(…) or /dev/stdin) hides what it runs from the guard. Run the commands themselves in this call, joined with && or newlines.";

const HIDDEN_REASON: &str = "The guard cannot see what this runs: a variable, a substitution, or the {} xargs or find fills in stands where the program or a git/gh subcommand goes. Spell the program and its subcommand out; for the git and gh calls ferrule replaces, the guard then names the ferrule command to run.";

const ALIAS_REASON: &str = "A git or gh alias that runs a shell command (!…), comes from the environment, or expands to a call ferrule replaces hides that call from the guard. Run the command itself; commits and pushes go through ferrule stage commit and ferrule pr push.";

pub(super) fn opaque_reason(o: Opaque) -> &'static str {
    match o {
        Opaque::AnsiC => {
            "ANSI-C quoting ($'…') is more than the guard follows. Use plain '…' or \"…\" quotes (printf for escapes) and retry."
        }
        Opaque::Arithmetic => {
            "An arithmetic command ((…)) is more than the guard follows. Use [ … ] or test, or $((…)) as a word (echo $((1+2)))."
        }
        Opaque::NestedSubstitution => {
            "A command substitution inside ${…} or $((…)) hides what it runs from the guard. Run the inner command on its own first (x=$(…)) and use the variable."
        }
    }
}

fn guard_list(
    commands: &[Command],
    cwd: &mut PathBuf,
    policy: Policy,
    depth: usize,
) -> Option<String> {
    if depth > MAX_DEPTH {
        return Some(DEEP_REASON.to_owned());
    }
    if let Some(reason) = written_then_run(commands, cwd) {
        return Some(reason);
    }
    for c in commands {
        if let Some(o) = c.opaque {
            return Some(opaque_reason(o).to_owned());
        }
        let Some((name, args)) = shell::program(&c.words) else {
            if let Some(reason) = owned_redirect(c, cwd) {
                return Some(reason);
            }
            continue;
        };
        // `git-commit`, as git's own libexec ships it, is `git commit`.
        let dashed: Vec<String>;
        let (name, args) = match name.strip_prefix("git-").filter(|s| !s.is_empty()) {
            Some(sub) => {
                dashed = std::iter::once(sub.to_owned())
                    .chain(args.iter().cloned())
                    .collect();
                ("git", dashed.as_slice())
            }
            None => (name, args),
        };
        // A directory the guard cannot follow is taken as `.ferrule`, the
        // strictest place a relative path can be checked from.
        if matches!(name, "popd") || (name == "cd" && args.iter().any(|a| a == "-")) {
            *cwd = PathBuf::from(UNKNOWN_CWD);
            continue;
        }
        if name == "cd" && cd_word(args).is_none() {
            *cwd = std::env::home_dir().unwrap_or_else(|| PathBuf::from(UNKNOWN_CWD));
            continue;
        }
        if matches!(name, "cd" | "pushd") {
            if let Some(t) = cd_word(args).filter(|t| globbed(t)) {
                return Some(format!(
                    "cd into a glob ('{t}') leaves the guard unsure where the commands after it run. cd to the directory by name."
                ));
            }
            if let Some(dir) = cd_target(cwd, args) {
                *cwd = dir;
            }
            continue;
        }
        let reason = owned_redirect(c, cwd)
            .or_else(|| fed_shell(c, name, args))
            .or_else(|| owned_edit(c, name, args, cwd))
            .or_else(|| owned_clean(name, args, cwd))
            .or_else(|| owned_removal(name, args, cwd))
            .or_else(|| stash_all(name, args))
            .or_else(|| git_config(name, args))
            .or_else(|| git_settings(c, name, args, cwd, policy, depth))
            .or_else(|| flood(c, name, args, cwd))
            .or_else(|| hidden(&c.words, name, args, policy))
            .or_else(|| replaced(name, args, policy))
            .or_else(|| behind(name, args, policy))
            .or_else(|| interpreted(c, name, args, policy))
            .or_else(|| nested(name, args, cwd, policy, depth));
        if reason.is_some() {
            return reason;
        }
    }
    None
}

/// What a program runs from its words, checked in turn: a shell's `-c`
/// script, `eval`'s words ([`shell::scripts`]), `find -exec … ;`.
fn nested(
    name: &str,
    args: &[String],
    cwd: &mut PathBuf,
    policy: Policy,
    depth: usize,
) -> Option<String> {
    for script in shell::scripts(name, args) {
        if let Some(reason) = guard_list(&shell::parse(&script), cwd, policy, depth + 1) {
            return Some(reason);
        }
    }
    let execs: Vec<Command> = shell::find_execs(name, args)
        .into_iter()
        .map(|words| Command {
            words,
            ..Command::default()
        })
        .collect();
    guard_list(&execs, cwd, policy, depth + 1)
}

/// Programs that run code given in their words, a heredoc or a file:
/// the guard reads that code for the git and gh calls it replaces.
const INTERPRETERS: &[&str] = &[
    "python",
    "python2",
    "python3",
    "perl",
    "node",
    "nodejs",
    "ruby",
    "php",
    "lua",
    "deno",
    "bun",
    "awk",
    "gawk",
    "mawk",
    "nawk",
    "sed",
    "Rscript",
    "tclsh",
    "pwsh",
    "osascript",
];

fn interpreter(name: &str) -> bool {
    INTERPRETERS.contains(&name)
        || name
            .strip_prefix("python")
            .is_some_and(|v| v.bytes().all(|b| b.is_ascii_digit() || b == b'.'))
}

/// The words of interpreter code as the guard reads it: runs of
/// characters a command word holds, split at quotes, commas, brackets and
/// the other punctuation code wraps a command in (`os.system("git push")`,
/// `["git", "push"]`).
fn code_words(code: &str) -> Vec<String> {
    code.split(|c: char| {
        !(c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/' | '=' | ':' | '@' | '+'))
    })
    .filter(|w| !w.is_empty())
    .map(str::to_owned)
    .collect()
}

/// A replaced git or gh call inside an interpreter's code: its inline code
/// (`-c`, `-e`, a script operand), a heredoc or a here-string it reads.
/// `sed 'e git push'` and `awk 'BEGIN{system("git push")}'` count.
fn interpreted(c: &Command, name: &str, args: &[String], policy: Policy) -> Option<String> {
    if !interpreter(name) {
        return None;
    }
    let bodies = c.redirects.iter().filter_map(|r| match r.kind {
        RedirectKind::HereDoc => r.body.as_deref(),
        RedirectKind::HereString => Some(r.target.as_str()),
        _ => None,
    });
    args.iter()
        .map(String::as_str)
        .chain(bodies)
        .find_map(|code| {
            let words = code_words(code);
            (0..words.len()).find_map(|i| {
                let w = shell::base(&words[i]);
                let rest = &words[i + 1..words.len().min(i + 24)];
                (w == "git" || w == "gh" || w.starts_with("git-"))
                    .then(|| replaced_as(w, rest, policy))
                    .flatten()
            })
        })
}

/// A file this command list writes (`>`, `>>`, `tee`) and then runs: as
/// a program by path, a shell's or an interpreter's script, or `source`.
/// The guard never sees what the file holds.
fn written_then_run(commands: &[Command], cwd: &Path) -> Option<String> {
    let mut written: Vec<&str> = Vec::new();
    for c in commands {
        written.extend(
            c.redirects
                .iter()
                .filter(|r| matches!(r.kind, RedirectKind::Write | RedirectKind::Append))
                .map(|r| r.target.as_str()),
        );
        if let Some(("tee", args)) = shell::program(&c.words) {
            written.extend(tee_targets(args).1);
        }
    }
    let same = |w: &str| {
        written
            .iter()
            .any(|t| *t == w || resolve(cwd, t) == resolve(cwd, w))
    };
    let runs = commands.iter().any(|c| {
        let Some((name, args)) = shell::program(&c.words) else {
            return false;
        };
        let by_path = c
            .words
            .iter()
            .find(|w| shell::base(w) == name)
            .is_some_and(|w| w.contains('/') && same(w));
        let operand = || args.iter().any(|a| !a.starts_with('-') && same(a));
        by_path
            || match name {
                "source" | "." => args.first().is_some_and(|a| same(a)),
                s if shell::SHELLS.contains(&s) => {
                    matches!(shell::shell_script(args), Script::File(f) if same(f))
                }
                n if interpreter(n) => operand(),
                _ => false,
            }
    });
    (!written.is_empty() && runs).then(|| WRITTEN_REASON.to_owned())
}

const WRITTEN_REASON: &str = "A file written and then run in one call hides what it runs from the guard. Run the commands themselves; for the git and gh calls ferrule replaces, the guard names the ferrule command to run.";

/// A shell, `source` or `eval` reading its script from stdin (a heredoc, a
/// here-string, a pipe, a redirect from a substitution) or from a
/// substitution (`<(…)`, `/dev/stdin`).
fn fed_shell(c: &Command, name: &str, args: &[String]) -> Option<String> {
    let streamed = |w: &str| {
        w.starts_with("<(") || w.starts_with("/dev/stdin") || w.starts_with("/dev/fd/") || w == "-"
    };
    let fed = c.fed
        || c.redirects.iter().any(|r| match r.kind {
            RedirectKind::HereDoc | RedirectKind::HereString => true,
            RedirectKind::Read => streamed(&r.target),
            _ => false,
        });
    let hit = match name {
        "eval" => fed || args.iter().any(|a| streamed(a)),
        "source" | "." => args.first().is_some_and(|a| streamed(a)),
        s if shell::SHELLS.contains(&s) => match shell::shell_script(args) {
            Script::Inline(_) => false,
            Script::File(f) => streamed(f),
            Script::Stdin => fed,
        },
        _ => false,
    };
    hit.then(|| FED_REASON.to_owned())
}

/// A word the guard cannot read: a variable, a substitution, a `{}` that
/// xargs or find fills in, a brace list (`{git,push}`).
fn unseen(word: &str) -> bool {
    word.contains(['$', '`']) || word.contains("{}") || brace_group(word).is_some()
}

/// A program, or a git/gh subcommand, the guard cannot read: as program
/// (also a glob, `/usr/bin/gi?`), a word alone or one that would be a
/// replaced git or gh call; a git or gh call whose subcommand is unseen
/// or, under xargs, missing.
fn hidden(words: &[String], name: &str, args: &[String], policy: Policy) -> Option<String> {
    let globbed_name = name.contains(['*', '?']) || (name.contains('[') && !name.starts_with('['));
    let blind = if unseen(name) || globbed_name {
        args.is_empty()
            || ["git", "gh"]
                .iter()
                .any(|p| replaced(p, args, policy).is_some())
    } else {
        match name {
            "git" => shell::git(args).map_or(shell::via_xargs(words), |g| unseen(g.sub)),
            "gh" => {
                let p = gh_positional(args);
                p.iter().take(2).any(|w| unseen(w)) || (shell::via_xargs(words) && p.len() < 2)
            }
            _ => false,
        }
    };
    blind.then(|| HIDDEN_REASON.to_owned())
}

/// Programs whose words are data, never a command they run.
const LITERAL: &[&str] = &[
    "echo", "printf", "grep", "egrep", "fgrep", "rg", "ag", "ack", "man", "which", "whereis",
    "type", "help", "info", "apropos", "tldr", "ferrule", "git", "gh", ":", "true", "false", "sed",
    "awk", "gawk", "mawk", "jq", "perl", "python", "python3", "node", "ruby",
];

/// A replaced git or gh call run by a program the guard does not know as a
/// wrapper: among its words (`strace -f git push`) or inside one of them
/// (`flock -c 'git push' lock`).
fn behind(name: &str, args: &[String], policy: Policy) -> Option<String> {
    if LITERAL.contains(&name) {
        return None;
    }
    args.iter().enumerate().find_map(|(i, a)| {
        let path_like = a
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '.' | '_' | '-'));
        let rest = &args[i + 1..];
        let among = if path_like {
            replaced_as(shell::base(a), rest, policy)
        } else {
            (unseen(a)
                && ["git", "gh"]
                    .iter()
                    .any(|p| replaced(p, rest, policy).is_some()))
            .then(|| HIDDEN_REASON.to_owned())
        };
        among.or_else(|| {
            a.contains(char::is_whitespace)
                .then(|| {
                    shell::parse(a).iter().find_map(|c| {
                        shell::program(&c.words).and_then(|(n, rest)| replaced_as(n, rest, policy))
                    })
                })
                .flatten()
        })
    })
}

/// [`replaced`] for a program named `git`, `gh` or `git-<sub>`.
fn replaced_as(name: &str, args: &[String], policy: Policy) -> Option<String> {
    match name {
        "git" | "gh" => replaced(name, args, policy),
        _ => {
            let sub = name.strip_prefix("git-").filter(|s| !s.is_empty())?;
            let full: Vec<String> = std::iter::once(sub.to_owned())
                .chain(args.iter().cloned())
                .collect();
            replaced("git", &full, policy)
        }
    }
}

/// Whether directory `dir` holds `.ferrule` or sits inside one.
fn holds_ferrule(dir: &Path) -> bool {
    dir.join(".ferrule").exists()
        || dir
            .components()
            .any(|c| c.as_os_str().eq_ignore_ascii_case(".ferrule"))
}

/// Whether `word`, from `cwd`, names a directory holding `.ferrule` (one
/// inside it does not), a glob that matches `.ferrule` itself, or the
/// current directory through
/// a variable or substitution the guard cannot read (`"$PWD"`).
fn reaches_ferrule(cwd: &Path, word: &str) -> bool {
    if matches!(word, "$PWD" | "${PWD}" | "$(…)" | "`…`") {
        return true;
    }
    braces(word).iter().any(|w| {
        let path = resolve(cwd, w);
        let (parent, file) = match (path.parent(), path.file_name().and_then(|f| f.to_str())) {
            (Some(p), Some(f)) => (p.to_path_buf(), f.to_owned()),
            _ => (path.clone(), String::new()),
        };
        path.join(".ferrule").exists()
            || (globbed(&file) && glob(&file, ".ferrule") && parent.join(".ferrule").exists())
    })
}

/// A program that deletes or moves whole trees run on one holding
/// `.ferrule` (`rm -rf .`, `rm -rf "$PWD"`), or `find -delete`/`-exec`
/// under one with no name test, or one an owned file matches.
fn owned_removal(name: &str, args: &[String], cwd: &Path) -> Option<String> {
    let hit = match name {
        "rm" | "rmdir" | "mv" | "shred" => args
            .iter()
            .skip_while(|a| a.as_str() != "--" && a.starts_with('-') && a.len() > 1)
            .filter(|a| a.as_str() != "--")
            .any(|a| reaches_ferrule(cwd, a)),
        "find" if has(args, &["-delete", "-exec", "-execdir", "-ok", "-okdir"]) => {
            let roots: Vec<&String> = args.iter().take_while(|a| !a.starts_with('-')).collect();
            let roots_hold = if roots.is_empty() {
                holds_ferrule(cwd)
            } else {
                roots.iter().any(|r| holds_ferrule(&resolve(cwd, r)))
            };
            let tests: Vec<&String> = args
                .windows(2)
                .filter(|w| {
                    matches!(
                        w[0].as_str(),
                        "-name" | "-iname" | "-path" | "-ipath" | "-wholename"
                    )
                })
                .map(|w| &w[1])
                .collect();
            let names_one = |t: &str| {
                let last = t.rsplit('/').next().unwrap_or(t);
                OWNED.iter().any(|(d, f)| glob(last, f) || glob(last, d))
            };
            roots_hold && (tests.is_empty() || tests.iter().any(|t| names_one(t)))
        }
        _ => false,
    };
    hit.then(|| format!("{OWNED_REASON} This removes or moves a tree holding .ferrule/, the board among it: name the paths inside it to remove instead."))
}

/// `git stash --all`: it stashes ignored files too, the board among them,
/// and removes them from the checkout.
fn stash_all(name: &str, args: &[String]) -> Option<String> {
    if name != "git" {
        return None;
    }
    let g = shell::git(args)?;
    let all = g.sub == "stash"
        && g.args.iter().any(|a| {
            a == "--all" || (a.starts_with('-') && !a.starts_with("--") && a.contains('a'))
        });
    all.then(|| format!("{OWNED_REASON} git stash --all stashes ignored files, .ferrule/state.db among them, and removes them: use git stash -u for untracked files."))
}

/// `git clean -x` where it would delete ferrule's ignored files: run in or
/// above `.ferrule`, on no pathspec or one that reaches it (resolved from
/// `-C` and the current directory, `:/` from the top), and without
/// `-e .ferrule`.
fn owned_clean(name: &str, args: &[String], cwd: &Path) -> Option<String> {
    if name != "git" {
        return None;
    }
    let g = shell::git(args)?;
    if g.sub != "clean" {
        return None;
    }
    let ignored = g.args.iter().any(|a| {
        a.starts_with('-') && !a.starts_with("--") && !a.starts_with("-e") && a.contains(['x', 'X'])
    });
    let mut excluded = false;
    let mut paths = Vec::new();
    let mut i = 0;
    while let Some(a) = g.args.get(i) {
        match a.as_str() {
            "-e" | "--exclude" => {
                excluded |= g.args.get(i + 1).is_some_and(|v| v.contains(".ferrule"));
                i += 1;
            }
            a if a.starts_with("--exclude=") || a.starts_with("-e") => {
                excluded |= a.contains(".ferrule");
            }
            a if !a.starts_with('-') => paths.push(a),
            _ => {}
        }
        i += 1;
    }
    let dir = g.dir.map_or_else(|| cwd.to_path_buf(), |d| resolve(cwd, d));
    let reach = |p: &str| {
        if p.starts_with(":/") || p.starts_with(":(top)") {
            return dir.ancestors().any(holds_ferrule);
        }
        matches!(p, "*" | "." | "./") && holds_ferrule(&dir)
            || p.contains(".ferrule")
            || reaches_ferrule(&dir, p)
    };
    let reaches = if paths.is_empty() {
        holds_ferrule(&dir)
    } else {
        paths.iter().any(|p| reach(p))
    };
    (ignored && !excluded && reaches).then(|| {
        format!("{OWNED_REASON} git clean -x deletes ignored files, .ferrule/state.db among them: add -e .ferrule.")
    })
}

/// Git settings no session sets: `core.hooksPath` points git away from
/// the hooks that refuse commits and pushes to the main branch.
const CONFIG_REFUSED: &[&str] = &["core.hookspath"];

/// A setting of [`CONFIG_REFUSED`], given to one call (`-c`,
/// `--config-env`) or written with `git config`; reading one stays open.
fn git_config(name: &str, args: &[String]) -> Option<String> {
    if name != "git" {
        return None;
    }
    let g = shell::git(args)?;
    let globals = &args[..args.len() - g.args.len() - 1];
    let refused = |kv: &str| {
        let key = kv.split('=').next().unwrap_or(kv).to_ascii_lowercase();
        CONFIG_REFUSED.contains(&key.as_str())
    };
    let mut hit = globals
        .windows(2)
        .any(|w| matches!(w[0].as_str(), "-c" | "--config-env") && refused(&w[1]))
        || globals
            .iter()
            .any(|a| a.strip_prefix("--config-env=").is_some_and(refused));
    if g.sub == "config" {
        let words: Vec<&str> = g.args.iter().map(String::as_str).collect();
        let reads = words
            .iter()
            .any(|a| matches!(*a, "--get" | "--get-all" | "--get-regexp" | "-l" | "--list"))
            || words
                .iter()
                .find(|a| !a.starts_with('-'))
                .is_some_and(|a| matches!(*a, "get" | "list"));
        hit |= !reads && words.iter().any(|a| !a.starts_with('-') && refused(a));
    }
    hit.then(|| {
        "Setting core.hooksPath turns off the git hooks that keep commits and pushes off the main branch. Leave it unset; ask the user if a hook is in the way.".to_owned()
    })
}

/// Environment variables whose value is a command git or a pager runs.
const RUNS_ENV: &[&str] = &[
    "GIT_SSH_COMMAND",
    "GIT_SSH",
    "GIT_PAGER",
    "PAGER",
    "GIT_EXTERNAL_DIFF",
    "GIT_EDITOR",
    "GIT_SEQUENCE_EDITOR",
    "EDITOR",
    "VISUAL",
    "GIT_ASKPASS",
    "SSH_ASKPASS",
];

const CONFIG_ENV_REASON: &str = "Git settings from the environment (GIT_CONFIG_PARAMETERS, GIT_CONFIG_COUNT/KEY/VALUE, GIT_CONFIG_GLOBAL) can define aliases and commands the guard cannot see. Run the git command without them.";

const AUTOCORRECT_REASON: &str = "help.autocorrect runs the git command a typo resembles, which the guard cannot tell. Spell the command out.";

const PULL_REASON: &str = "git pull without --ff-only can make a merge commit or rebase, which is ferrule's to land. Use git pull --ff-only, or git fetch and ferrule's commands.";

/// What the guard says about `code` run as a command line of its own.
fn guard_code(code: &str, cwd: &Path, policy: Policy, depth: usize) -> Option<String> {
    let mut cwd = cwd.to_path_buf();
    guard_list(&shell::parse(code), &mut cwd, policy, depth + 1)
}

/// The command lines git settings and variables hand to git, checked as
/// commands of their own: `GIT_SSH_COMMAND=…`, `-c core.pager=…`,
/// `git config diff.external …`, `rebase -x`, `bisect run`, `submodule
/// foreach`; settings from the environment and `help.autocorrect` are
/// refused, `git pull` without `--ff-only`, and an alias from the git
/// configuration that runs a shell or a replaced call.
fn git_settings(
    c: &Command,
    name: &str,
    args: &[String],
    cwd: &Path,
    policy: Policy,
    depth: usize,
) -> Option<String> {
    fn assignment(w: &str) -> Option<(&str, &str)> {
        w.split_once('=').filter(|(k, _)| {
            !k.is_empty()
                && k.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                && !k.as_bytes()[0].is_ascii_digit()
        })
    }
    let exported = matches!(
        name,
        "export" | "declare" | "typeset" | "readonly" | "local"
    );
    let before = c.words.len().saturating_sub(args.len() + 1);
    let assigned = c.words[..before]
        .iter()
        .chain(args.iter().filter(|_| exported))
        .filter_map(|w| assignment(w));
    for (k, v) in assigned {
        if k.starts_with("GIT_CONFIG") {
            return Some(CONFIG_ENV_REASON.to_owned());
        }
        if RUNS_ENV.contains(&k)
            && let Some(r) = guard_code(v, cwd, policy, depth)
        {
            return Some(r);
        }
    }
    if name != "git" {
        return None;
    }
    let g = shell::git(args)?;
    let globals = &args[..args.len() - g.args.len() - 1];
    let mut settings: Vec<(String, &str)> = Vec::new();
    for (i, w) in globals.iter().enumerate() {
        if w == "-c"
            && let Some((k, v)) = globals.get(i + 1).and_then(|kv| kv.split_once('='))
        {
            settings.push((k.to_ascii_lowercase(), v));
        }
        if w == "--config-env" || w.starts_with("--config-env=") {
            return Some(CONFIG_ENV_REASON.to_owned());
        }
    }
    if g.sub == "config" {
        let words: Vec<&String> = g.args.iter().filter(|a| !a.starts_with('-')).collect();
        let words = match words.first().map(|w| w.as_str()) {
            Some("set") => &words[1..],
            _ => &words[..],
        };
        if let [k, v, ..] = words {
            settings.push((k.to_ascii_lowercase(), v.as_str()));
        }
    }
    for (k, v) in &settings {
        if k == "help.autocorrect" && !matches!(*v, "0" | "never" | "false") {
            return Some(AUTOCORRECT_REASON.to_owned());
        }
        if let Some(r) = guard_code(v.trim_start_matches('!'), cwd, policy, depth) {
            return Some(r);
        }
    }
    let after = |flag: &str| {
        g.args.iter().enumerate().find_map(|(i, a)| {
            if a == flag {
                g.args.get(i + 1).map(String::as_str)
            } else {
                a.strip_prefix(&format!("{flag}="))
            }
        })
    };
    let code: Option<String> = match g.sub {
        "rebase" => after("-x").or_else(|| after("--exec")).map(str::to_owned),
        "bisect" if g.args.first().is_some_and(|a| a == "run") => Some(g.args[1..].join(" ")),
        "submodule" if g.args.first().is_some_and(|a| a == "foreach") => {
            g.args.iter().skip(1).find(|a| !a.starts_with('-')).cloned()
        }
        "pull" if !g.args.iter().any(|a| a == "--ff-only") => {
            return Some(PULL_REASON.to_owned());
        }
        _ => None,
    };
    if let Some(r) = code.and_then(|code| guard_code(&code, cwd, policy, depth)) {
        return Some(r);
    }
    let dir = g.dir.map_or_else(|| cwd.to_path_buf(), |d| resolve(cwd, d));
    config_alias(g.sub, g.args, &dir, policy)
}

/// A git alias the configuration defines for `sub`: one running a shell
/// (`!…`) is refused, another is checked expanded.
fn config_alias(sub: &str, args: &[String], dir: &Path, policy: Policy) -> Option<String> {
    if !dir.is_dir() || sub.starts_with('-') {
        return None;
    }
    let value = crate::git::out(dir, &["config", "--get", &format!("alias.{sub}")])?;
    if value.trim_start().starts_with('!') {
        return Some(ALIAS_REASON.to_owned());
    }
    let full: Vec<String> = shell::parse(&value)
        .into_iter()
        .next()
        .map(|c| c.words)
        .unwrap_or_default()
        .into_iter()
        .chain(args.iter().cloned())
        .collect();
    replaced("git", &full, policy).map(|_| ALIAS_REASON.to_owned())
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
                    "Whole-file cat of a .md file. Run: ferrule file headings {a} to see headings, then ferrule file section '<heading>' {a} for just what's needed. Genuinely need the whole document? Use the Read tool instead."
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

const OWNED_REASON: &str = "Only ferrule writes .ferrule/state.db and .ferrule/ferrule.lock, and only the user edits the settings this guard comes from (.ferrule/ferrule.toml, .claude/settings.json, .claude/settings.local.json). Use ferrule board log|move|set for the board, ferrule board show|board to read it, ferrule setup install for the lock; ask the user to change a setting.";

/// Programs that may name ferrule's files: they only list or read them.
const OWNED_READERS: &[&str] = &[
    "ls",
    "stat",
    "file",
    "test",
    "[",
    "cat",
    "head",
    "tail",
    "less",
    "wc",
    "grep",
    "rg",
    "diff",
    "cmp",
    "sha256sum",
    "md5sum",
    "ferrule",
];

/// Readers only in some forms: [`readonly_program`] tells which.
const OWNED_READERS_IF_READONLY: &[&str] = &["git", "sed", "awk", "gawk", "mawk", "find"];

fn owned_redirect(c: &Command, cwd: &Path) -> Option<String> {
    c.redirects
        .iter()
        .any(|r| {
            matches!(r.kind, RedirectKind::Write | RedirectKind::Append)
                && names_owned(cwd, &r.target)
        })
        .then(|| OWNED_REASON.to_owned())
}

/// Any program but a reader that names ferrule's files, in its words or in
/// the heredoc or here-string it reads: a writer, an interpreter running
/// inline code, or a command feeding the name to `xargs`.
fn owned_edit(c: &Command, name: &str, args: &[String], cwd: &Path) -> Option<String> {
    let reader = OWNED_READERS.contains(&name)
        || (OWNED_READERS_IF_READONLY.contains(&name) && readonly_program(name, args));
    let hit = if name == "tee" {
        tee_targets(args).1.iter().any(|t| names_owned(cwd, t))
    } else {
        // `git clean -e .ferrule` keeps the directory.
        let excluded =
            |i: usize| name == "git" && i > 0 && matches!(args[i - 1].as_str(), "-e" | "--exclude");
        !reader
            && (args
                .iter()
                .enumerate()
                .any(|(i, a)| !excluded(i) && names_owned(cwd, a))
                || c.redirects.iter().any(|r| match r.kind {
                    RedirectKind::HereDoc => r.body.as_deref().is_some_and(|b| names_owned(cwd, b)),
                    RedirectKind::HereString => names_owned(cwd, &r.target),
                    _ => false,
                }))
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
        "gh" => gh_positional(args),
        _ => return None,
    };
    Some(std::iter::once(name).chain(rest).collect())
}

/// `gh` options, global or a command's, whose value is the next word.
const GH_VALUED: &[&str] = &[
    "-R",
    "--repo",
    "--hostname",
    "-b",
    "--body",
    "-F",
    "--body-file",
    "-t",
    "--title",
    "-B",
    "--base",
    "-H",
    "--head",
    "-a",
    "--assignee",
    "-l",
    "--label",
    "-m",
    "--milestone",
    "-p",
    "--project",
    "-r",
    "--reviewer",
    "-T",
    "--template",
    "-q",
    "--jq",
    "--json",
    "-S",
    "--search",
    "-s",
    "--state",
    "-L",
    "--limit",
    "-A",
    "--author",
    "-j",
    "--job",
    "-X",
    "--method",
    "--color",
];

/// A `gh` call's words that are neither options nor their values.
fn gh_positional(args: &[String]) -> Vec<&str> {
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(a) = args.get(i) {
        if GH_VALUED.contains(&a.as_str()) {
            i += 1;
        } else if !a.starts_with('-') {
            out.push(a.as_str());
        }
        i += 1;
    }
    out
}

fn replaced(name: &str, args: &[String], policy: Policy) -> Option<String> {
    let words = verb_words(name, args)?;
    if has_prefix(name, args, policy.allow) || allowed(&words, policy.allow) {
        return None;
    }
    if let Some(reason) = alias(name, args, policy) {
        return Some(reason);
    }
    if words.starts_with(&["git", "commit"]) && args.iter().any(|a| a == "--dry-run") {
        return None;
    }
    // Hook-only: a `permissions.deny` prefix rule cannot tell a branch from
    // a tag.
    if words.starts_with(&["git", "update-ref"]) {
        return update_ref(args);
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
            "A writing 'gh api' call is ferrule's to run here. Use: ferrule pr feedback reply <pr> <file> [<stage> <sha>] for review threads, ferrule issue post / ferrule pr publish for the rest. Re-open it with [guard] allow = [\"gh api\"] in .ferrule/ferrule.toml if the project needs it.".to_owned()
        });
    }
    // A run's summary and its `--json` fields are short; its logs flood.
    // Hook-only: a `permissions.deny` prefix rule cannot tell them apart.
    if words.starts_with(&["gh", "run", "view"]) {
        let logs = args
            .iter()
            .any(|a| a == "--log" || a == "--log-failed" || a.starts_with("--log="));
        return logs.then(|| {
            "A raw CI log floods the context. Use: ferrule ci failed [<branch>] [--run <id>] [--job <name>] [--context <n>]: the failed job's error lines, each with the n lines after it. A run's summary or --json fields (gh run view <id>) stay allowed.".to_owned()
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

/// Whether the command's verb words start with one of `[guard] allow`'s
/// (`gh -R o/r issue create` re-opened by `gh issue create`).
fn allowed(words: &[&str], allow: &[String]) -> bool {
    allow.iter().any(|p| {
        let p: Vec<&str> = p.split_whitespace().collect();
        !p.is_empty() && words.starts_with(&p)
    })
}

/// A git alias set with `-c alias.<x>=…` or `git config alias.<x> …`, or a
/// gh alias set with `gh alias set|import`, that runs a shell command or
/// expands to a replaced call; and a call of such a `-c` alias with its
/// arguments.
fn alias(name: &str, args: &[String], policy: Policy) -> Option<String> {
    let deny = || Some(ALIAS_REASON.to_owned());
    let expands = |prog: &str, value: &str| {
        let words = shell::parse(value)
            .into_iter()
            .next()
            .map(|c| c.words)
            .unwrap_or_default();
        value.trim_start().starts_with('!') || replaced(prog, &words, policy).is_some()
    };
    if name == "gh" {
        let p = gh_positional(args);
        return match p.as_slice() {
            ["alias", "import", ..] => deny(),
            ["alias", "set", _, value, ..]
                if has(args, &["-s", "--shell"]) || expands("gh", value) =>
            {
                deny()
            }
            _ => None,
        };
    }
    if name != "git" {
        return None;
    }
    let g = shell::git(args)?;
    let globals = &args[..args.len() - g.args.len() - 1];
    // Name (lower-cased, as git looks it up) and value; none from the
    // environment.
    let mut defs: Vec<(String, Option<&str>)> = Vec::new();
    let is_alias = |k: &str| k.to_ascii_lowercase().starts_with("alias.");
    for (i, w) in globals.iter().enumerate() {
        let next = globals.get(i + 1).map(String::as_str);
        match w.as_str() {
            "-c" => {
                if let Some((k, v)) = next
                    .and_then(|kv| kv.split_once('='))
                    .filter(|(k, _)| is_alias(k))
                {
                    defs.push((k[6..].to_ascii_lowercase(), Some(v)));
                }
            }
            "--config-env" if next.is_some_and(is_alias) => defs.push((String::new(), None)),
            w if w.strip_prefix("--config-env=").is_some_and(is_alias) => {
                defs.push((String::new(), None));
            }
            _ => {}
        }
    }
    if g.sub == "config"
        && let Some(i) = g.args.iter().position(|a| is_alias(a))
        && let Some(value) = g.args.get(i + 1)
    {
        defs.push((g.args[i][6..].to_ascii_lowercase(), Some(value)));
    }
    for (alias, value) in defs {
        let Some(value) = value else {
            return deny();
        };
        if expands("git", value) {
            return deny();
        }
        if alias == g.sub.to_ascii_lowercase() {
            let full: Vec<String> = shell::parse(value)
                .into_iter()
                .next()
                .map(|c| c.words)
                .unwrap_or_default()
                .into_iter()
                .chain(g.args.iter().cloned())
                .collect();
            if let Some(reason) = replaced("git", &full, policy) {
                return Some(reason);
            }
        }
    }
    None
}

/// `git update-ref` on a branch (`refs/heads/…`, `HEAD`), or on refs read
/// from stdin.
fn update_ref(args: &[String]) -> Option<String> {
    let g = shell::git(args)?;
    let mut target = None;
    let mut i = 0;
    while let Some(a) = g.args.get(i) {
        match a.as_str() {
            "-m" => i += 1,
            a if !a.starts_with('-') => {
                target = Some(a);
                break;
            }
            _ => {}
        }
        i += 1;
    }
    let branch = g.args.iter().any(|a| a == "--stdin")
        || target.is_some_and(|r| r.starts_with("refs/heads/") || r == "HEAD" || r == "@");
    branch.then(|| format!("'git update-ref' on a branch is ferrule's to run in this project: {COMMIT_WHY}. Use: {COMMIT_INSTEAD}"))
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
pub(super) fn has(args: &[String], flags: &[&str]) -> bool {
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

/// Whether the command's words start with one of `prefixes`' words.
pub(super) fn has_prefix(name: &str, args: &[String], prefixes: &[String]) -> bool {
    let words: Vec<&str> = std::iter::once(name)
        .chain(args.iter().map(String::as_str))
        .collect();
    prefixes.iter().any(|p| {
        let p: Vec<&str> = p.split_whitespace().collect();
        !p.is_empty() && words.starts_with(&p)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ut_owned_paths() {
        assert!(is_owned(".ferrule/state.db"));
        assert!(is_owned("/x/.ferrule/state.db-wal"));
        assert!(is_owned(".ferrule/ferrule.lock"));
        assert!(is_owned(".ferrule/ferrule.toml"));
        assert!(is_owned("/x/.claude/settings.json"));
        assert!(is_owned(".claude/settings.local.json"));
        assert!(!is_owned("/tmp/state.db"));
        assert!(!is_owned("node/settings.json"));
        assert!(!is_owned(".ferrule/runs/x/plan.md"));
    }

    fn guard(cmd: &str) -> Option<String> {
        shell(
            cmd,
            Path::new("/nonexistent-ferrule-guard"),
            Policy { allow: &[] },
        )
    }

    /// Commands the guard refuses outright, one per rule.
    fn refused() -> Vec<String> {
        let mut out: Vec<String> = REPLACED.iter().map(|r| format!("{} 7", r.raw)).collect();
        out.extend(
            [
                "git commit -m x",
                "git update-ref refs/heads/main abc",
                "gh run view 9 --log",
                "gh api -X POST repos/o/r/issues",
                "rm .ferrule/state.db",
                "sqlite3 .ferrule/state.db x",
            ]
            .map(String::from),
        );
        out
    }

    /// Ways to run a command without naming it first.
    const WRAPPERS: &[&str] = &[
        "{}",
        "timeout 60 {}",
        "timeout -s KILL 5 {}",
        "timeout --signal=KILL 5 {}",
        "nice -n 5 {}",
        "nice -5 {}",
        "env {}",
        "env -u X A=b {}",
        "exec -a n {}",
        "coproc {}",
        "command {}",
        "builtin command {}",
        "sudo -u root {}",
        "nohup {}",
        "time -p {}",
        "stdbuf -oL {}",
        "ionice -c 3 {}",
        "strace -f {}",
        "echo a | xargs {}",
        "xargs -n 1 -I x {}",
        "find . -exec {} \\;",
        "find . -name x -execdir {} +",
        "sh -c '{}'",
        "bash -c \"{}\"",
        "bash -lc '{}'",
        "bash -c 'bash -c \"{}\"'",
        "eval {}",
        "eval \"{}\"",
        "builtin eval {}",
        "bash <<EOF\n{}\nEOF",
        "bash <<'EOF'\n{}\nEOF",
        "bash <<< \"{}\"",
        "echo '{}' | sh",
        "bash <(echo '{}')",
        "{ {}; }",
        "( {} )",
        "if true; then {}; fi",
        "true && {}",
        "! {}",
        "flock -c '{}' /tmp/l",
        "env -S '{}'",
        "x=$'a' {}",
        "(( 1 << 2 )); {}",
        "echo ${x:-$(true)}; {}",
    ];

    /// The program word written otherwise.
    fn spellings(cmd: &str) -> Vec<String> {
        let (p, rest) = cmd.split_once(' ').unwrap_or((cmd, ""));
        let mut out = vec![
            format!("'{p}' {rest}"),
            format!("\"{p}\" {rest}"),
            format!("{}\\{} {rest}", &p[..1], &p[1..]),
            format!("{}\"\"{} {rest}", &p[..1], &p[1..]),
            format!("/usr/bin/{p} {rest}"),
            format!("\"$(which {p})\" {rest}"),
            format!("$(command -v {p}) {rest}"),
        ];
        if let Some(sub) = cmd.strip_prefix("git ") {
            let (s, r) = sub.split_once(' ').unwrap_or((sub, ""));
            out.push(format!("/usr/lib/git-core/git-{s} {r}"));
            out.push(format!("git -C . -c core.pager=cat --no-pager {sub}"));
        }
        if let Some(sub) = cmd.strip_prefix("gh ") {
            out.push(format!("gh -R o/r {sub}"));
            out.push(format!("gh --repo o/r {sub}"));
            out.push(format!("gh --repo=o/r {sub}"));
        }
        out
    }

    #[test]
    fn ut_refused_however_wrapped_or_spelled() {
        let mut missed = Vec::new();
        for cmd in refused() {
            let forms = std::iter::once(cmd.clone()).chain(spellings(&cmd));
            for form in forms {
                for w in WRAPPERS {
                    let full = w.replace("{}", &form);
                    if guard(&full).is_none() {
                        missed.push(full);
                    }
                }
            }
        }
        assert!(missed.is_empty(), "allowed:\n{}", missed.join("\n"));
    }

    #[test]
    fn ut_everyday_commands_pass() {
        for cmd in [
            "echo $(git rev-parse HEAD)",
            "cd \"$(git rev-parse --show-toplevel)\" && ls",
            "git log --oneline -5",
            "git merge-base main HEAD",
            "git update-ref refs/tags/v1 abc",
            "git -c core.pager=cat log -1",
            "git -c alias.st=status st",
            "timeout 60 cargo test",
            "nice -n 5 cargo build",
            "bash -c 'cargo test 2>&1 | tail -5'",
            "find . -name '*.rs' -exec grep -l foo {} \\;",
            "echo a b | xargs -n 1 echo",
            "echo 'git push'",
            "grep -rn 'git commit' .",
            "gh -R o/r pr list",
            "gh pr checks 3 --repo o/r",
            "cat <<EOF\nx\nEOF",
            "python3 - <<EOF\nprint(1)\nEOF",
            "for i in 1 2; do echo $i; done",
            "echo $((1 + 2)) ${x:-y}",
            "command -v git",
            "env | grep PATH",
            "rm -rf target",
            "diff <(ls a) <(ls b)",
            "\"$EDITOR\" notes.txt",
            "git clean -fdx target",
            "sudo -u root ls",
        ] {
            assert_eq!(guard(cmd), None, "{cmd}");
        }
    }

    #[test]
    fn ut_git_aliases_and_commit_makers() {
        for cmd in [
            "git -c alias.ci=commit ci -m x",
            "git -c 'alias.p=!git push' status",
            "git -c alias.x=status -c 'alias.y=!sh' x",
            "git -c ALIAS.ci=commit ci",
            "git --config-env=alias.ci=CMD ci",
            "git -c alias.u=update-ref u refs/heads/main abc",
            "git config alias.ci commit",
            "git config --global alias.p '!git push'",
            "gh alias set pc 'pr create'",
            "gh alias set x 'git push' --shell",
            "gh alias import aliases.yml",
            "git merge feat",
            "git cherry-pick abc",
            "git revert abc",
            "git am 0001.patch",
            "git commit-tree abc -m x",
            "git send-pack origin main",
            "git update-ref -m x HEAD abc",
            "git update-ref --stdin",
            "git $sub -m x",
            "git \"$@\"",
            "echo commit | xargs git",
            "echo create | xargs gh pr",
            "$(echo git push)",
            "$GIT push",
            "{git,push}",
            "/usr/bin/gi? push",
            "git {commit,-m,x}",
        ] {
            assert!(guard(cmd).is_some(), "{cmd}");
        }
    }

    #[test]
    fn ut_ferrule_dir_and_globbed_names() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join(".ferrule")).unwrap();
        let g = |cmd: &str| shell(cmd, root.path(), Policy { allow: &[] });
        for cmd in [
            "rm -rf .ferrule",
            "rm -rf ./.ferrule/",
            "mv .ferrule /tmp/x",
            "rm -rf .fer*",
            "rm .ferrule/sta{te,x}.db",
            "rm .ferrule/STATE.DB",
            "rm .FERRULE/ferrule.LOCK",
            "rm .ferrule/x/../state.db",
            "git -C .ferrule clean -fdx",
            "git clean -fdx",
            "cd .fer* && rm state.db",
            "cd .f?rrule",
            "cd \"$(pwd)/.ferrule\" && rm state.db",
            "pushd .ferrule && rm state.db",
        ] {
            assert!(g(cmd).is_some(), "{cmd}");
        }
        for cmd in [
            "ls .ferrule",
            "rm .ferrule/runs/demo/x.md",
            "git clean -fdx -e .ferrule",
            "git clean -fd",
            "cd .ferrule && ls",
        ] {
            assert_eq!(g(cmd), None, "{cmd}");
        }
    }

    #[test]
    fn ut_globs_and_braces() {
        assert!(glob(".fer*", ".ferrule"));
        assert!(!glob("*", ".ferrule"));
        assert!(glob("sta[s-u]e.DB", "state.db"));
        assert!(!glob("sta[!t]e.db", "state.db"));
        assert_eq!(braces("a{b,c{d,e}}f"), ["abf", "acdf", "acef"]);
        assert_eq!(braces("x{1..3}"), ["x*"]);
        assert_eq!(braces("${HOME}/x"), ["${HOME}/x"]);
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
