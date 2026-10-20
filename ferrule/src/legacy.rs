//! The command names before the topic restructure, kept for one release:
//! [`rewrite`] turns an old command path into its new one before clap parses
//! the arguments, so old calls run and help shows only the new names.

/// `[old path, new path]`, longest old path first.
const PATHS: &[(&[&str], &[&str])] = &[
    (&["card", "commit"], &["stage", "commit"]),
    (&["card", "board"], &["board", "list"]),
    (&["card"], &["board"]),
    (&["run", "approve"], &["gate", "approve"]),
    (&["run", "next"], &["gate", "next"]),
    (&["run", "sha-check"], &["stage", "verify"]),
    (&["run", "stage-stop"], &["stage", "stop"]),
    (&["run", "replan"], &["stage", "replan"]),
    (&["run", "gauntlet"], &["ci", "gauntlet"]),
    (&["run", "ci"], &["ci", "wait"]),
    (&["pr", "failed"], &["ci", "failed"]),
    (&["spec", "sections"], &["file", "headings"]),
    (&["spec", "section"], &["file", "section"]),
    (&["issue", "file"], &["issue", "post"]),
    (&["uninstall"], &["setup", "uninstall"]),
];

/// Subcommands of `guide`; any other word after it is a topic.
const GUIDE: &[&str] = &["show", "list", "check", "help"];

/// The arguments with an old command path replaced by its new one; new
/// paths pass through unchanged.
#[must_use]
pub fn rewrite(args: Vec<String>) -> Vec<String> {
    let Some(at) = command_index(&args) else {
        return args;
    };
    // `ferrule help <path>` names a path too
    let at = if args[at] == "help" { at + 1 } else { at };
    let (head, rest) = args.split_at(at.min(args.len()));
    let mut out = head.to_vec();
    out.extend(rewrite_path(rest));
    out
}

/// The index of the first word that is not a global option or its value.
fn command_index(args: &[String]) -> Option<usize> {
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--format" => i += 2,
            a if a.starts_with('-') => i += 1,
            _ => return Some(i),
        }
    }
    None
}

fn rewrite_path(rest: &[String]) -> Vec<String> {
    let words: Vec<&str> = rest.iter().map(String::as_str).collect();
    match words.as_slice() {
        ["install", tail @ ..] => {
            let (check, tail): (Vec<&str>, Vec<&str>) = tail.iter().partition(|&&a| a == "--check");
            let sub = if check.is_empty() { "install" } else { "check" };
            owned(["setup", sub].iter().chain(&tail))
        }
        ["guide", tail @ ..] => guide(tail),
        _ => {
            for (old, new) in PATHS {
                if words.starts_with(old) {
                    return owned(new.iter().chain(&words[old.len()..]));
                }
            }
            rest.to_vec()
        }
    }
}

/// `guide <topic>` is `guide show <topic>`; `--list` and `--check` are the
/// `list` and `check` subcommands.
fn guide(tail: &[&str]) -> Vec<String> {
    for (flag, sub) in [("--list", "list"), ("--check", "check")] {
        if tail.contains(&flag) {
            let rest = tail.iter().filter(|&&a| a != flag);
            return owned(["guide", sub].iter().chain(rest));
        }
    }
    let first = tail.iter().find(|a| !a.starts_with('-'));
    let help = tail.iter().any(|a| matches!(*a, "-h" | "--help"));
    match first {
        Some(word) if !GUIDE.contains(word) => owned(["guide", "show"].iter().chain(tail)),
        None if !help && !tail.is_empty() => owned(["guide", "show"].iter().chain(tail)),
        _ => owned(["guide"].iter().chain(tail)),
    }
}

fn owned<'a>(words: impl IntoIterator<Item = &'a &'a str>) -> Vec<String> {
    words.into_iter().map(|w| (*w).to_owned()).collect()
}

#[cfg(test)]
mod tests {
    use super::rewrite;

    fn r(line: &str) -> String {
        let args = std::iter::once("ferrule")
            .chain(line.split(' ').filter(|w| !w.is_empty()))
            .map(str::to_owned)
            .collect();
        rewrite(args)[1..].join(" ")
    }

    #[test]
    fn ut_old_paths_map_to_new_ones() {
        let cases = [
            ("card commit demo.s1 -m x", "stage commit demo.s1 -m x"),
            ("card board", "board list"),
            ("card log demo note -- x", "board log demo note -- x"),
            ("run approve demo gate2", "gate approve demo gate2"),
            (
                "run next demo gate2 status=ready",
                "gate next demo gate2 status=ready",
            ),
            ("run sha-check demo s1 abc", "stage verify demo s1 abc"),
            ("run stage-stop demo s1", "stage stop demo s1"),
            ("run replan demo start", "stage replan demo start"),
            ("run gauntlet demo", "ci gauntlet demo"),
            ("run ci demo", "ci wait demo"),
            ("run init demo", "run init demo"),
            ("pr failed --run 7", "ci failed --run 7"),
            ("pr push demo", "pr push demo"),
            ("spec sections a.md", "file headings a.md"),
            ("spec section Status a.md", "file section Status a.md"),
            ("spec id MB-R-001", "spec id MB-R-001"),
            ("issue file demo --split", "issue post demo --split"),
            ("install --force", "setup install --force"),
            ("install --check", "setup check"),
            ("uninstall --purge", "setup uninstall --purge"),
            ("guide gate2 --slug demo", "guide show gate2 --slug demo"),
            ("guide --slug demo gate2", "guide show --slug demo gate2"),
            ("guide --list", "guide list"),
            ("guide --check", "guide check"),
            ("guide show gate2", "guide show gate2"),
            ("guide list", "guide list"),
            ("guide --help", "guide --help"),
            ("--format md card show demo", "--format md board show demo"),
            ("help card log", "help board log"),
        ];
        for (old, new) in cases {
            assert_eq!(r(old), new, "{old}");
        }
    }
}
