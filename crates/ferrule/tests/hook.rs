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
fn it_shell_guard_refuses_floods_and_writes_to_ferrules_files() {
    let p = project();
    check(
        "shell",
        p.path(),
        &[
            ("cat notes.md", "deny"),
            ("cat notes.md | head -5", "allow"),
            ("cat big.txt", "deny"),
            ("cat big.txt > copy.txt", "allow"),
            ("git diff", "deny"),
            ("git diff main...HEAD", "deny"),
            ("git diff --stat", "allow"),
            ("git diff main -- src", "allow"),
            ("git show main:src/x.rs", "allow"),
            ("find . -type f", "deny"),
            ("find . -type f -name '*.rs'", "allow"),
            ("ls -la", "allow"),
            ("echo x >> .ferrule/state.db", "deny"),
            ("sqlite3 .ferrule/state.db 'select 1'", "deny"),
            ("rm -f /x/.ferrule/state.db-wal", "deny"),
            ("echo x | tee .ferrule/ferrule.lock", "deny"),
            ("cp backup.db .ferrule/state.db", "deny"),
            ("echo x >> .ferrule/runs/demo/review.md", "allow"),
            ("echo 'x' > .ferrule/ferrule.toml", "allow"),
            ("ferrule card log demo green", "allow"),
            ("echo 'cat notes.md'", "allow"),
            ("echo $(cat notes.md)", "allow"),
        ],
    );
}

#[test]
fn it_shell_guard_names_the_ferrule_command_for_raw_git_and_gh() {
    let p = project();
    check(
        "shell",
        p.path(),
        &[
            ("git commit -m x", "deny"),
            ("git -C sub commit -m x", "deny"),
            ("cd /tmp && git commit -m x", "deny"),
            ("env git commit -m x", "deny"),
            ("git commit --dry-run -m x", "allow"),
            ("git push -u origin feat/demo", "deny"),
            ("git worktree add ../wt -b x", "deny"),
            ("git worktree list", "allow"),
            ("git log --oneline -5", "allow"),
            ("git status --short", "allow"),
            ("git fetch -q origin main", "allow"),
            ("gh issue create --title t --body b", "deny"),
            ("gh issue comment 12 --body-file f", "deny"),
            ("gh issue list --state open", "allow"),
            ("gh issue view 12", "deny"),
            ("gh pr view 34 | head", "deny"),
            ("gh pr create --draft", "deny"),
            ("gh pr ready 34", "deny"),
            ("gh pr checks 34", "allow"),
            ("gh run view 99 --log-failed", "deny"),
            ("gh run view 99 --job 7 --log", "deny"),
            ("gh run view 99", "allow"),
            ("gh run view 99 --json jobs,status", "allow"),
            ("gh api repos/o/r/pulls", "allow"),
            (
                "gh api -X POST repos/o/r/pulls/1/comments -f body=x",
                "deny",
            ),
            ("gh api graphql -F id=x -f query=y", "deny"),
        ],
    );
    let payload = json!({
        "tool_name": "Bash",
        "tool_input": {"command": "gh pr create --draft"},
        "cwd": p.path(),
    });
    let out = run_hook("shell", p.path(), payload.to_string().as_bytes());
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    let reason = v["hookSpecificOutput"]["permissionDecisionReason"]
        .as_str()
        .unwrap();
    assert!(reason.contains("Use: ferrule pr open <slug>"), "{reason}");
}

#[test]
fn it_shell_guard_honours_the_projects_allow_list() {
    let p = project();
    std::fs::create_dir(p.path().join(".ferrule")).unwrap();
    std::fs::write(
        p.path().join(".ferrule/ferrule.toml"),
        "[guard]\nallow = [\"gh api\", \"gh issue create\"]\n",
    )
    .unwrap();
    check(
        "shell",
        p.path(),
        &[
            ("gh api -X POST repos/o/r/issues -f title=x", "allow"),
            ("gh issue create --title t", "allow"),
            ("gh issue edit 3 --title t", "deny"),
        ],
    );
}

#[test]
fn it_shell_guard_resolves_cat_after_cd() {
    let p = project();
    std::fs::create_dir(p.path().join("sub")).unwrap();
    std::fs::write(p.path().join("sub/inner.md"), "# y\n").unwrap();
    check(
        "shell",
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
            ("echo x >> .ferrule/runs/demo/review.md", "allow"),
            (
                "cat > .ferrule/runs/demo/review.verdict.md <<EOF\nx\nEOF",
                "allow",
            ),
            ("echo x > .ferrule/runs/demo/review.md", "deny"),
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
            hook("attribution", p.path(), &cmd),
            Got::Rewrite(want),
            "{cmd}"
        );
    }
    check(
        "attribution",
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
        "attribution",
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
    let out = run_hook("shell", p.path(), b"not json");
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty());
}

#[test]
fn it_other_tools_pass() {
    let p = tempfile::tempdir().unwrap();
    let payload = json!({"tool_name": "Read", "tool_input": {"file_path": "x"}, "cwd": p.path()});
    let out = run_hook("shell", p.path(), payload.to_string().as_bytes());
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

fn git(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new("git")
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
        .output()
        .unwrap()
}

fn git_hook(dir: &Path, args: &[&str], stdin: &str) -> std::process::Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_ferrule"))
        .args(["hook", "git"])
        .args(args)
        .current_dir(dir)
        .env("CLAUDE_PROJECT_DIR", dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

#[test]
fn it_git_hooks_refuse_main_and_strip_trailers() {
    let p = tempfile::tempdir().unwrap();
    let repo = p.path();
    assert!(git(repo, &["init", "-q", "-b", "main"]).status.success());
    assert_eq!(git_hook(repo, &["pre-commit"], "").status.code(), Some(1));
    assert!(
        git(repo, &["checkout", "-q", "-b", "feat/x"])
            .status
            .success()
    );
    assert_eq!(git_hook(repo, &["pre-commit"], "").status.code(), Some(0));

    let push_main = "refs/heads/feat/x abc refs/heads/main def\n";
    assert_eq!(
        git_hook(repo, &["pre-push", "origin", "u"], push_main)
            .status
            .code(),
        Some(1)
    );
    let push_feat = "refs/heads/feat/x abc refs/heads/feat/x def\n";
    assert_eq!(
        git_hook(repo, &["pre-push", "origin", "u"], push_feat)
            .status
            .code(),
        Some(0)
    );

    let msg = repo.join("MSG");
    std::fs::write(&msg, format!("Fix it\n\n{}: A <a@b>\n", co())).unwrap();
    assert_eq!(
        git_hook(repo, &["commit-msg", msg.to_str().unwrap()], "")
            .status
            .code(),
        Some(0)
    );
    assert_eq!(std::fs::read_to_string(&msg).unwrap(), "Fix it\n");
}

#[test]
fn it_session_start_names_unfinished_cards() {
    let p = tempfile::tempdir().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_ferrule"))
        .args(["hook", "session-start"])
        .env("CLAUDE_PROJECT_DIR", p.path())
        .current_dir(p.path())
        .output()
        .unwrap();
    assert!(out.stdout.is_empty());
    let new = Command::new(env!("CARGO_BIN_EXE_ferrule"))
        .args(["card", "new", "demo", "--state", "inprogress"])
        .env("CLAUDE_PROJECT_DIR", p.path())
        .current_dir(p.path())
        .output()
        .unwrap();
    assert!(new.status.success());
    let out = Command::new(env!("CARGO_BIN_EXE_ferrule"))
        .args(["hook", "session-start"])
        .env("CLAUDE_PROJECT_DIR", p.path())
        .current_dir(p.path())
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("demo inprogress\n") && text.contains("ferrule guide resume"),
        "{text}"
    );
}
