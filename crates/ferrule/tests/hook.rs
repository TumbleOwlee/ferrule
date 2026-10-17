use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use serde_json::{Value, json};

/// What the hook decided for a Bash call.
#[derive(Debug, PartialEq, Eq)]
enum Got {
    Allow,
    Deny,
    Rewrite(String),
}

fn run_hook(kind: &str, cwd: &Path, payload: &[u8]) -> std::process::Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_ferrule"))
        .args(["hook", kind])
        .current_dir(cwd)
        .env("CLAUDE_PROJECT_DIR", cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(payload).unwrap();
    child.wait_with_output().unwrap()
}

fn hook(kind: &str, cwd: &Path, cmd: &str) -> Got {
    let payload = json!({
        "tool_name": "Bash",
        "tool_input": {"command": cmd, "description": "d"},
        "cwd": cwd,
    });
    let out = run_hook(kind, cwd, payload.to_string().as_bytes());
    assert_eq!(out.status.code(), Some(0), "{cmd}");
    if out.stdout.is_empty() {
        return Got::Allow;
    }
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    let h = &v["hookSpecificOutput"];
    if h["permissionDecision"] == "deny" {
        return Got::Deny;
    }
    assert_eq!(h["updatedInput"]["description"], "d", "other fields kept");
    Got::Rewrite(h["updatedInput"]["command"].as_str().unwrap().to_owned())
}

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let lines: String = (1..=100).map(|n| n.to_string() + "\n").collect();
    std::fs::write(dir.path().join("big.txt"), lines).unwrap();
    std::fs::write(dir.path().join("notes.md"), "# x\n").unwrap();
    dir
}

fn check(kind: &str, cwd: &Path, cases: &[(&str, &str)]) {
    for (cmd, want) in cases {
        let got = hook(kind, cwd, cmd);
        let ok = match *want {
            "allow" => got == Got::Allow,
            "deny" => got == Got::Deny,
            _ => unreachable!(),
        };
        assert!(ok, "{kind} {cmd:?}: expected {want}, got {got:?}");
    }
}

#[test]
fn it_shell_guard_refuses_floods_card_edits_and_github_views() {
    let p = project();
    check(
        "pre-tool-use",
        p.path(),
        &[
            ("cat notes.md", "deny"),
            ("cat notes.md | head -5", "allow"),
            ("cat big.txt", "deny"),
            ("cat big.txt > copy.txt", "allow"),
            ("gh issue view 12", "deny"),
            ("gh pr view 34", "deny"),
            ("gh pr view 34 | head", "deny"),
            ("git diff", "deny"),
            ("git diff main...HEAD", "deny"),
            ("git diff --stat", "allow"),
            ("git diff main -- src", "allow"),
            ("git show main:src/x.rs", "allow"),
            ("find . -type f", "deny"),
            ("find . -type f -name '*.rs'", "allow"),
            ("ls -la", "allow"),
            ("echo x >> .claude/tasks/open/demo.s2.md", "deny"),
            (
                "grep a b | tee -a /x/.claude/tasks/inprogress/demo.md",
                "deny",
            ),
            ("cat >> .claude/tasks/done/demo.s0.md <<EOF\nx\nEOF", "deny"),
            ("echo x | tee --append .claude/tasks/open/a.s1.md", "deny"),
            ("echo x | tee -a -- .claude/tasks/open/a.s1.md", "deny"),
            ("echo x >> /tmp/done/notes.md", "allow"),
            ("echo x >> .claude/tasks/artifacts/demo/review.md", "allow"),
            ("sh .claude/scripts/card-log.sh demo green", "allow"),
            (
                "mv .claude/tasks/open/demo.s2.md .claude/tasks/done/",
                "deny",
            ),
            ("mv /tmp/x.md .claude/tasks/done/demo.s9.md", "deny"),
            ("mv notes.md docs/notes.md", "allow"),
            ("sh .claude/scripts/card-move.sh demo.s2 done", "allow"),
            ("echo 'cat notes.md'", "allow"),
            ("echo $(cat notes.md)", "allow"),
        ],
    );
}

#[test]
fn it_shell_guard_resolves_cat_after_cd() {
    let p = project();
    std::fs::create_dir(p.path().join("sub")).unwrap();
    std::fs::write(p.path().join("sub/inner.md"), "# y\n").unwrap();
    check(
        "pre-tool-use",
        p.path(),
        &[
            ("cd sub && cat inner.md", "deny"),
            ("cd nosuch; cat notes.md", "deny"),
            ("cat inner.md", "allow"),
        ],
    );
}

#[test]
fn it_reviewer_guard_allows_reads_and_the_review_files_only() {
    let p = project();
    check(
        "reviewer",
        p.path(),
        &[
            ("git checkout src/x.rs", "deny"),
            ("git diff main...HEAD -- src", "allow"),
            ("git grep -n MB-R-161 main -- docs/specs", "allow"),
            ("echo x >> .claude/tasks/artifacts/demo/review.md", "allow"),
            (
                "cat > .claude/tasks/artifacts/demo/review.verdict.md <<EOF\nx\nEOF",
                "allow",
            ),
            ("echo x > .claude/tasks/artifacts/demo/review.md", "deny"),
            ("echo x > src/lib.rs", "deny"),
            ("rm -rf target", "deny"),
            ("cargo test 2>&1 | tail -5", "allow"),
            ("cargo fmt", "deny"),
            ("cargo fmt --check", "allow"),
            ("grep -rn '=>' src > /dev/null", "allow"),
            ("echo $(rm -f x)", "deny"),
            ("for f in a b; do rm $f; done", "deny"),
            ("awk '$2 > 5 { print $1 }' f", "allow"),
            ("awk '{ print > \"out\" }' f", "deny"),
            ("sed -n '1,5p' f", "allow"),
            ("sed -i s/a/b/ f", "deny"),
            ("gh pr comment 3 --body x", "deny"),
            ("gh pr diff 3", "allow"),
            ("gh api repos/o/r/pulls", "allow"),
            ("gh api -X POST repos/o/r/issues", "deny"),
            ("git stash list", "allow"),
            ("git stash", "deny"),
            ("git branch -D x", "deny"),
            ("git branch", "allow"),
            ("python3 -c 'print(1)'", "deny"),
            ("bash -c 'ls'", "deny"),
        ],
    );
}

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

#[test]
fn it_shell_guard_refuses_commits_and_pushes_to_main() {
    let p = tempfile::tempdir().unwrap();
    let repo = p.path().join("r");
    std::fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["commit", "-q", "--allow-empty", "-m", "init"]);
    let wt = p.path().join("wt");
    git(
        &repo,
        &[
            "worktree",
            "add",
            "-q",
            wt.to_str().unwrap(),
            "-b",
            "feat/demo",
        ],
    );
    check(
        "pre-tool-use",
        &repo,
        &[
            ("git commit -m x", "deny"),
            ("git commit -F - <<EOF\nx\nEOF", "deny"),
            ("git commit --dry-run -m x", "allow"),
            ("git push origin main", "deny"),
            ("git push origin HEAD:main", "deny"),
            ("git push", "deny"),
        ],
    );
    let other = format!("git -C {} commit -m x", repo.display());
    check(
        "pre-tool-use",
        &wt,
        &[
            ("git commit -m x", "allow"),
            ("git push -u origin feat/demo", "allow"),
            (&other, "deny"),
        ],
    );
}

/// Attribution text is assembled at run time so this file carries none
/// literally.
fn co() -> String {
    ["Co", "Authored", "By"].join("-")
}

fn gen_with() -> String {
    ["Generated", "with"].join(" ")
}

#[test]
fn it_attribution_is_stripped_not_refused() {
    let p = tempfile::tempdir().unwrap();
    let (co, gw) = (co(), gen_with());
    let co_lower = co.to_lowercase();
    let cases = [
        (
            format!("git commit -m \"fix: x\n\n{co}: Someone <a@b.c>\""),
            "git commit -m 'fix: x'".to_owned(),
        ),
        (
            format!("gh pr create --title t --body \"{gw} a tool\""),
            "gh pr create --title t --body ''".to_owned(),
        ),
        (
            format!("git commit -F - <<EOF\nfix: x\n\n🤖 {gw} a tool\nEOF"),
            "git commit -F - <<EOF\nfix: x\nEOF".to_owned(),
        ),
        (
            format!("git commit --trailer \"{co}: X <a@b.c>\" -m x"),
            "git commit -m x".to_owned(),
        ),
        (
            format!("git commit --trailer {co_lower}:X -m x"),
            "git commit -m x".to_owned(),
        ),
        (
            format!("git commit --trailer={co_lower}:X -m x"),
            "git commit -m x".to_owned(),
        ),
        (
            format!("git commit -F - <<EOF\nfix: x\n\n- 🤖 {gw} a tool\nEOF"),
            "git commit -F - <<EOF\nfix: x\nEOF".to_owned(),
        ),
        (
            format!("git commit -F - <<EOF\nfix: x\n\n*{gw} a tool*\nEOF"),
            "git commit -F - <<EOF\nfix: x\nEOF".to_owned(),
        ),
        (
            format!("git commit -F - <<EOF\nfix: x\n\n🤖 *{gw} a tool*\nEOF"),
            "git commit -F - <<EOF\nfix: x\nEOF".to_owned(),
        ),
        (
            format!("git commit --trailer {co_lower}=A -m x"),
            "git commit -m x".to_owned(),
        ),
        (
            format!("gh pr create --title t --body \"x\\n\\n{co_lower}: A\""),
            "gh pr create --title t --body 'x'".to_owned(),
        ),
        (
            format!("git commit -F - <<EOF\nfix: x\n\n> {gw} a tool\nEOF"),
            "git commit -F - <<EOF\nfix: x\nEOF".to_owned(),
        ),
    ];
    for (cmd, want) in cases {
        assert_eq!(
            hook("pre-tool-use", p.path(), &cmd),
            Got::Rewrite(want),
            "{cmd}"
        );
    }
    check(
        "pre-tool-use",
        p.path(),
        &[
            ("git commit -m \"fix: x\"", "allow"),
            (
                "git commit -m \"docs: images regenerated with images/capture.py\"",
                "allow",
            ),
            (
                "git commit -m \"fix: hook no longer matches generated with in prose\"",
                "allow",
            ),
        ],
    );
}

#[test]
fn it_attribution_in_a_message_file_is_refused() {
    let p = tempfile::tempdir().unwrap();
    std::fs::write(
        p.path().join("msg.txt"),
        format!("fix: x\n\n{}: A <a@b>\n", co()),
    )
    .unwrap();
    std::fs::write(p.path().join("ok.txt"), "fix: x\n").unwrap();
    check(
        "pre-tool-use",
        p.path(),
        &[
            ("git commit -F msg.txt", "deny"),
            ("gh pr create --body-file msg.txt", "deny"),
            ("git commit -F ok.txt", "allow"),
        ],
    );
}

#[test]
fn it_unreadable_payload_blocks() {
    let p = tempfile::tempdir().unwrap();
    let out = run_hook("pre-tool-use", p.path(), b"not json");
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty());
}

#[test]
fn it_other_tools_pass() {
    let p = tempfile::tempdir().unwrap();
    let payload = json!({"tool_name": "Read", "tool_input": {"file_path": "x"}, "cwd": p.path()});
    let out = run_hook("pre-tool-use", p.path(), payload.to_string().as_bytes());
    assert_eq!(out.status.code(), Some(0));
    assert!(out.stdout.is_empty());
}

#[test]
fn it_reviewer_refuses_the_configured_mutating_commands() {
    let p = tempfile::tempdir().unwrap();
    std::fs::create_dir(p.path().join(".ferrule")).unwrap();
    std::fs::write(
        p.path().join(".ferrule/ferrule.toml"),
        "[hook.readonly]\nmutating = [\"npm install\"]\n",
    )
    .unwrap();
    check(
        "reviewer",
        p.path(),
        &[
            ("npm install left-pad", "deny"),
            ("npm ls", "allow"),
            ("env CI=1 npm install", "deny"),
        ],
    );
}
