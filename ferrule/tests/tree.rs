//! The command tree: one topic per command, one operation per subcommand,
//! and the names before the restructure still running for one release.

mod golden;

use golden::Run;

/// Every command path, as `--help` lists them.
const TREE: &str = "\
setup
setup install
setup check
setup uninstall
guide
guide show
guide list
guide check
run
run init
run reconcile
run rebuild
run cleanup
run metrics
run export
gate
gate approve
gate next
stage
stage commit
stage verify
stage stop
stage replan
ci
ci gauntlet
ci wait
ci failed
board
board list
board show
board field
board new
board set
board move
board log
board vocab
board import
spec
spec id
spec related
spec collisions
spec render
file
file headings
file section
prompt
prompt render
prompt lines
prompt show
issue
issue view
issue request
issue body
issue post
pr
pr view
pr body
pr open
pr push
pr publish
pr feedback
pr feedback route
pr feedback fetch
pr feedback reply
hook
hook shell
hook attribution
hook reviewer
hook session-start
hook git
";

/// The subcommands `ferrule <path> --help` lists, `help` left out.
fn subcommands(path: &[&str]) -> Vec<String> {
    let out = Run::new(path.iter().copied().chain(["--help"])).output();
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.lines()
        .skip_while(|l| *l != "Commands:")
        .skip(1)
        .take_while(|l| !l.is_empty())
        .filter_map(|l| l.split_whitespace().next())
        .filter(|name| *name != "help")
        .map(str::to_owned)
        .collect()
}

fn walk(path: &mut Vec<String>, lines: &mut String) {
    let words: Vec<&str> = path.iter().map(String::as_str).collect();
    for name in subcommands(&words) {
        path.push(name);
        lines.push_str(&path.join(" "));
        lines.push('\n');
        walk(path, lines);
        path.pop();
    }
}

#[test]
fn it_tree_is_the_agreed_one() {
    let mut lines = String::new();
    walk(&mut Vec::new(), &mut lines);
    assert_eq!(lines, TREE);
}

/// Stdout and exit code of one call.
fn result(args: &[&str]) -> (String, Option<i32>) {
    let out = Run::new(args.iter().copied()).output();
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        out.status.code(),
    )
}

#[test]
fn it_old_names_run_the_new_commands() {
    let cases: &[(&[&str], &[&str])] = &[
        (&["card", "vocab"], &["board", "vocab"]),
        (&["guide", "--list"], &["guide", "list"]),
        (&["guide", "principles"], &["guide", "show", "principles"]),
        (&["help", "card", "log"], &["board", "log", "--help"]),
        (&["run", "next", "--help"], &["gate", "next", "--help"]),
        (
            &["run", "sha-check", "--help"],
            &["stage", "verify", "--help"],
        ),
        (
            &["run", "stage-stop", "--help"],
            &["stage", "stop", "--help"],
        ),
        (&["run", "ci", "--help"], &["ci", "wait", "--help"]),
        (&["pr", "failed", "--help"], &["ci", "failed", "--help"]),
        (
            &["card", "commit", "--help"],
            &["stage", "commit", "--help"],
        ),
        (
            &["spec", "sections", "--help"],
            &["file", "headings", "--help"],
        ),
        (&["issue", "file", "--help"], &["issue", "post", "--help"]),
        (&["install", "--help"], &["setup", "install", "--help"]),
        (&["uninstall", "--help"], &["setup", "uninstall", "--help"]),
    ];
    for (old, new) in cases {
        assert_eq!(result(old), result(new), "{old:?}");
    }
}

#[test]
fn it_hides_the_old_names() {
    let top = subcommands(&[]);
    for old in ["card", "install", "uninstall"] {
        assert!(!top.iter().any(|c| c == old), "{old} is listed");
    }
}
