//! The issue and pr groups against a fake `gh`, with the workflow's
//! expected outputs.

mod golden;

use std::path::Path;

use golden::{FakeGh, GH_FIELDS, Run, fixtures, gh_json};

fn fixture(name: &str) -> String {
    std::fs::read_to_string(fixtures().join(name)).unwrap()
}

fn ferrule(gh: &FakeGh, project: &Path, args: &[&str]) -> Run {
    gh.on(Run::new(args))
        .env("CLAUDE_PROJECT_DIR", project)
        .env("FERRULE_NOW", "2026-10-05T09:00")
        .cwd(project)
}

/// `gh issue view <n>` serves the fixture's issue.
fn serve_issue(gh: &FakeGh, n: &str, name: &str) {
    gh.serve(
        &["issue", "view", n, "--json", GH_FIELDS],
        &gh_json(&fixture(name)),
    );
}

#[test]
fn it_request_reads_back_the_original_request() {
    let gh = FakeGh::new();
    let p = tempfile::tempdir().unwrap();
    for (n, name) in [
        ("7", "issue/plain.txt"),
        ("8", "issue/assembled.txt"),
        ("71", "issue/slice.txt"),
        ("9", "issue/new.txt"),
        ("10", "issue/nonbehavior.txt"),
    ] {
        serve_issue(&gh, n, name);
    }
    let req = |args: &[&str], golden: &str| {
        let mut all = vec!["issue", "request"];
        all.extend_from_slice(args);
        ferrule(&gh, p.path(), &all).golden(golden);
    };
    req(&["7"], "issue-request-plain");
    req(&["8"], "issue-request-assembled");
    req(&["8", "--spec"], "issue-request-spec");
    req(&["71", "--epic"], "issue-request-epic");
    req(&["7", "--epic"], "issue-request-epic-none");
    req(&["71", "--spec"], "issue-request-slice-spec");
    req(&["7", "--spec"], "issue-request-spec-plain");
    req(&["9"], "issue-request-new");
    req(&["9", "--spec"], "issue-request-new-spec");
    req(&["10"], "issue-request-nonbehavior");
    assert_eq!(
        ferrule(&gh, p.path(), &["issue", "request", "71", "--slice-line"]).text(),
        "**Part of epic** #70 · **Slice** 1 of 3 · **Depends on** none\nexit=0"
    );
    assert_eq!(
        ferrule(
            &gh,
            p.path(),
            &["issue", "request", "7", "--spec", "--epic"]
        )
        .text(),
        "exit=2"
    );
    assert_eq!(
        ferrule(&gh, p.path(), &["issue", "request", "404"]).text(),
        "exit=1"
    );
}

#[test]
fn it_view_prints_compact_text() {
    let gh = FakeGh::new();
    let p = tempfile::tempdir().unwrap();
    serve_issue(&gh, "7", "issue/plain.txt");
    gh.serve(
        &["pr", "view", "22", "--json", GH_FIELDS],
        &gh_json(&fixture("pr/ready.txt")),
    );
    let plain = fixture("issue/plain.txt");
    let before_comments = plain.split("--- ").next().unwrap();
    assert_eq!(
        ferrule(&gh, p.path(), &["issue", "view", "7"]).text(),
        format!("{before_comments}--- 0 comment(s) ---\nexit=0")
    );
    let pr = ferrule(&gh, p.path(), &["pr", "view", "22"]).text();
    assert!(
        pr.starts_with("#22 Reconnect dropped Modbus clients [OPEN]\nhttps://github.com/o/r/pull/22\nby someone\n\nReady · lane full"),
        "{pr}"
    );
    assert!(pr.ends_with("--- 0 comment(s) ---\nexit=0"), "{pr}");
    assert!(gh.log().contains(&format!("pr view 22 --json {GH_FIELDS}")));
}

const THREADS: &str = r#"{"data":{"repository":{"pullRequest":{"state":"OPEN","isDraft":true,"reviewThreads":{"nodes":[
{"id":"T1","isResolved":false,"path":"src/a.rs","line":42,"originalLine":40,"comments":{"nodes":[{"databaseId":11,"author":{"login":"al"},"createdAt":"2026-10-05T08:00:00Z","body":"rename it"}]}},
{"id":"T2","isResolved":true,"path":"src/b.rs","line":1,"originalLine":1,"comments":{"nodes":[{"databaseId":12,"author":{"login":"al"},"createdAt":"2026-10-05T08:01:00Z","body":"done"}]}},
{"id":"T3","isResolved":false,"path":"src/c.rs","line":null,"originalLine":null,"comments":{"nodes":[{"databaseId":13,"author":null,"createdAt":"2026-10-05T08:02:00Z","body":"why?"}]}}
]}}}}}"#;

#[test]
fn it_feedback_fetches_threads_and_replies() {
    let gh = FakeGh::new();
    let p = tempfile::tempdir().unwrap();
    let query = "query=query($owner: String!, $repo: String!, $pr: Int!) {\n  repository(owner: $owner, name: $repo) {";
    gh.serve(
        &[
            "api",
            "graphql",
            "-F",
            "owner={owner}",
            "-F",
            "repo={repo}",
            "-F",
            "pr=22",
            "-f",
            query,
        ],
        THREADS,
    );
    let file = p.path().join("threads.md");
    let f = file.to_str().unwrap();
    assert_eq!(
        ferrule(&gh, p.path(), &["pr", "feedback", "fetch", "22", f]).text(),
        format!("pr=22 state=open draft=true threads=2 file={f}\nexit=0")
    );
    let text = std::fs::read_to_string(&file).unwrap();
    assert_eq!(
        text,
        "# PR feedback — fetched 2026-10-05T09:00\n## thread T1\ncomment: 11\npath: src/a.rs:42\n[al @ 2026-10-05T08:00:00Z]\nrename it\nreply:\n\n## thread T3\ncomment: 13\npath: src/c.rs:?\n[unknown @ 2026-10-05T08:02:00Z]\nwhy?\nreply:\n\n"
    );
    let filled = text
        .replacen("reply:\n", "reply: fixed\n  - renamed to `b`\n", 1)
        .replacen(
            "why?\nreply:\n",
            "why?\nreply: declined\n  - the spec asks for it\n",
            1,
        );
    std::fs::write(&file, filled).unwrap();
    assert_eq!(
        ferrule(&gh, p.path(), &["pr", "feedback", "reply", "22", f]).text(),
        "exit=2"
    );
    assert_eq!(
        ferrule(
            &gh,
            p.path(),
            &["pr", "feedback", "reply", "22", f, "s2", "abc1234"]
        )
        .text(),
        "pr=22 fixed=1 declined=1 open=1\nexit=0"
    );
    let log = gh.log();
    assert!(
        log.contains("api repos/{owner}/{repo}/pulls/22/comments/11/replies -f body=Fixed in s2 (abc1234)\n- renamed to `b`\n"),
        "{log}"
    );
    assert!(
        log.contains("api graphql -F id=T1 -f query=mutation"),
        "{log}"
    );
    assert!(
        log.contains("api repos/{owner}/{repo}/pulls/22/comments/13/replies -f body=Not changed — the spec asks for it\n"),
        "{log}"
    );
    assert!(
        !log.contains("id=T3"),
        "a declined thread stays open: {log}"
    );
}

#[test]
fn it_failed_prints_the_error_lines() {
    let gh = FakeGh::new();
    let p = tempfile::tempdir().unwrap();
    gh.serve(
        &[
            "run",
            "list",
            "--branch",
            "feat/x",
            "--status",
            "failure",
            "--limit",
            "1",
            "--json",
            "databaseId",
        ],
        r#"[{"databaseId":5}]"#,
    );
    gh.serve(
        &["run", "view", "5", "--json", "jobs"],
        r#"{"jobs":[{"databaseId":8,"conclusion":"success"},{"databaseId":9,"conclusion":"failure"}]}"#,
    );
    gh.serve(
        &["run", "view", "5", "--job", "9", "--log-failed"],
        "test\tCompiling x\ntest\terror[E0308]: mismatched types\ntest\tPost job cleanup failed\ntest\ttest a ... FAILED\n",
    );
    assert_eq!(
        ferrule(&gh, p.path(), &["ci", "failed", "feat/x"]).text(),
        "test\terror[E0308]: mismatched types\ntest\ttest a ... FAILED\nexit=0"
    );
    assert_eq!(
        ferrule(&gh, p.path(), &["ci", "failed", "feat/none"]).text(),
        "exit=1"
    );
}

#[test]
fn it_failed_takes_a_run_a_job_and_context() {
    let gh = FakeGh::new();
    let p = tempfile::tempdir().unwrap();
    gh.serve(
        &["run", "view", "6", "--json", "jobs"],
        r#"{"jobs":[{"databaseId":8,"name":"test","conclusion":"failure"},{"databaseId":9,"name":"lint (stable)","conclusion":"failure"}]}"#,
    );
    gh.serve(
        &["run", "view", "6", "--job", "9", "--log-failed"],
        "lint\tChecking x\nlint\terror: unused import\nlint\t  --> src/a.rs:1\nlint\tdone\nlint\twarning: x\nlint\terror: second\nlint\tnote: y\n",
    );
    assert_eq!(
        ferrule(
            &gh,
            p.path(),
            &[
                "ci",
                "failed",
                "--run",
                "6",
                "--job",
                "LINT",
                "--context",
                "1"
            ]
        )
        .text(),
        "lint\terror: unused import\nlint\t  --> src/a.rs:1\n--\nlint\terror: second\nlint\tnote: y\nexit=0"
    );
    assert_eq!(
        ferrule(
            &gh,
            p.path(),
            &["ci", "failed", "--run", "6", "--job", "docs"]
        )
        .text(),
        "exit=1"
    );
}
