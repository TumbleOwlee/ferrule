mod golden;

use std::path::Path;

use golden::{Run, check, fixtures};

const NOW: &str = "2026-10-05T09:00";

/// A project whose state database holds the fixture board.
fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let board = fixtures().join("board");
    let got = ferrule(dir.path(), &["card", "import", board.to_str().unwrap()]).text();
    assert_eq!(got, "import=ok cards=13\nexit=0");
    dir
}

fn ferrule(project: &Path, args: &[&str]) -> Run {
    Run::new(args)
        .env("CLAUDE_PROJECT_DIR", project)
        .env("FERRULE_NOW", NOW)
        .cwd(project)
}

/// `card log`, then the card's last log line when it was accepted, as the
/// workflow's tests read it back with `tail -1`.
fn log_tail(project: &Path, card: &str, args: &[&str]) -> String {
    let mut all = vec!["card", "log", card];
    all.extend_from_slice(args);
    let out = ferrule(project, &all).text();
    if out != "exit=0" {
        return out;
    }
    let shown = ferrule(project, &["card", "show", card]).text();
    let last = shown.lines().rev().nth(1).unwrap_or_default().to_owned();
    format!("{last}\nexit=0")
}

#[test]
fn it_field_reads_frontmatter_log_and_pass_sha() {
    let p = project();
    let mut got = String::new();
    for f in [
        "lane",
        "issue",
        "split",
        "log:cov",
        "log:review",
        "pass-sha",
        "nosuch",
    ] {
        let out = ferrule(p.path(), &["card", "field", "demo", f]).output();
        got.push_str(f);
        got.push('=');
        got.push_str(&String::from_utf8_lossy(&out.stdout));
        got.push('\n');
    }
    let none = ferrule(p.path(), &["card", "field", "nosuch", "lane"]).text();
    got.push_str(&none);
    check("card-field", &got);
}

#[test]
fn it_log_appends_validated_lines() {
    let p = project();
    let p = p.path();
    let cases: &[(&str, &str, &[&str])] = &[
        (
            "card-log-ok",
            "demo.s1",
            &[
                "spec-delta",
                "MB-R-161",
                "changed",
                "origin=user",
                "quote=keep retrying",
                "--",
                "retry",
                "forever",
            ],
        ),
        (
            "card-log-parent",
            "demo",
            &["stop", "kind=approval", "gate=plan"],
        ),
        (
            "card-log-test-red-noid",
            "demo.s1",
            &["test-red", "--", "ut_status_line"],
        ),
        (
            "card-log-minor-moved",
            "demo.s1",
            &["minor-moved=wording -> s3", "--", "the", "template"],
        ),
        (
            "card-log-files-add",
            "demo.s1",
            &[
                "files+=CONTRIBUTING.md, a.md",
                "user=approved",
                "reason=review",
            ],
        ),
        (
            "card-log-note",
            "demo.s1",
            &["note", "--", "renamed", "\"attr-a\"", "to", "attr-b"],
        ),
        (
            "card-log-note-parent",
            "demo",
            &["note", "--", "plan", "amended", "after", "review"],
        ),
    ];
    for (name, card, args) in cases {
        check(name, &log_tail(p, card, args));
    }
    let x135 = "x".repeat(135);
    check(
        "card-log-accept-160",
        &log_tail(p, "demo.s1", &["stopped", "--", &x135]),
    );
}

#[test]
fn it_log_rejects_and_leaves_the_card_unchanged() {
    let p = project();
    let p = p.path();
    let before = ferrule(p, &["card", "show", "demo.s1"]).text();
    let x160 = "x".repeat(160);
    let cases: &[(&str, &[&str])] = &[
        ("card-log-reject-event", &["demo.s1", "frobnicate"]),
        ("card-log-reject-key", &["demo.s1", "green", "foo=1"]),
        (
            "card-log-reject-scope",
            &["demo.s1", "stop", "kind=question"],
        ),
        ("card-log-reject-value", &["demo.s1", "checks=maybe"]),
        (
            "card-log-reject-length",
            &["demo.s1", "stopped", "--", &x160],
        ),
        ("card-log-reject-stop-kind", &["demo", "stop", "gate=plan"]),
        (
            "card-log-reject-stop-delta",
            &["demo", "stop", "kind=delta"],
        ),
        (
            "card-log-reject-stop-replan",
            &["demo", "stop", "kind=replan"],
        ),
        ("card-log-reject-event-quote", &["demo.s1", "commit=a\"b"]),
        ("card-log-no-card", &["nosuch.s9", "green"]),
        ("card-log-note-empty", &["demo.s1", "note"]),
        (
            "card-log-reject-note-typed",
            &[
                "demo",
                "note",
                "--",
                "plan-review",
                "findings",
                "count=3",
                "round=1",
            ],
        ),
        (
            "card-log-reject-note-review",
            &["demo", "note", "--", "branch-review", "findings", "count=2"],
        ),
        ("card-log-read-no-text", &["demo.s1", "read", "path=f1.rs"]),
    ];
    for (name, args) in cases {
        let mut all = vec!["card", "log"];
        all.extend_from_slice(args);
        ferrule(p, &all).golden(name);
    }
    assert_eq!(ferrule(p, &["card", "show", "demo.s1"]).text(), before);
    let waived = ferrule(p, &["card", "log", "demo", "ci-waived"]).output();
    assert_eq!(waived.status.code(), Some(2));
}

#[test]
fn it_log_read_budget_and_commit_state() {
    let p = project();
    let p = p.path();
    for k in 1..=3 {
        let path = format!("path=f{k}.rs");
        let got = ferrule(
            p,
            &["card", "log", "demo.s2", "read", &path, "--", "the", "plan"],
        )
        .text();
        assert_eq!(got, "exit=0");
    }
    ferrule(
        p,
        &[
            "card",
            "log",
            "demo.s2",
            "read",
            "path=f4.rs",
            "--",
            "still",
            "thin",
        ],
    )
    .golden("card-log-read-budget");
    new(p, &["demo.s3", "--state", "inprogress"]);
    ferrule(p, &["card", "log", "demo.s3", "commit=abc1234"])
        .golden("card-log-reject-commit-inprogress");
    let shown = ferrule(p, &["card", "show", "demo.s3"]).text();
    assert!(!shown.contains("commit="));
    assert_eq!(
        log_tail(p, "demo.s1", &["commit=abc1234"]),
        "2026-10-05T09:00 commit=abc1234\nexit=0"
    );
}

fn new(p: &Path, args: &[&str]) {
    let mut all = vec!["card", "new"];
    all.extend_from_slice(args);
    assert_eq!(ferrule(p, &all).text(), "exit=0", "{args:?}");
}

#[test]
fn it_move_follows_the_board_rules() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    new(
        p,
        &[
            "ss",
            "--state",
            "inprogress",
            "branch=feat/ss",
            "lane=full",
            "gate2=2026-10-03",
        ],
    );
    new(p, &["ss.s0", "title=Spec", "blocked-by=[]"]);
    new(p, &["ss.s1", "title=One", "blocked-by=[ss.s0]"]);
    new(p, &["ss.s2", "title=Two", "blocked-by=[s1]"]);
    let mv = |card: &str, to: &str| ferrule(p, &["card", "move", card, to]).text();
    let refused = |from: &str, to: &str, reason: &str| {
        let hint = match reason {
            "transition" => "see the card table in `ferrule guide board` for allowed moves",
            "blocked-by" => "move the cards in its blocked-by to done first",
            "commit" => "commit the stage with `ferrule card commit <card> -m <msg>` first",
            "unverified" => {
                "run `ferrule run gauntlet <slug>` so a gauntlet=pass covers the commit sha"
            }
            _ => "approve gate 2 first: `ferrule run approve <slug> gate2`",
        };
        format!("card-move=refused from={from} to={to} reason={reason} hint={hint}\nexit=2")
    };

    assert_eq!(
        mv("ss.s1", "inprogress"),
        refused("open", "inprogress", "blocked-by")
    );
    assert_eq!(mv("ss.s0", "inprogress"), "exit=0");
    assert_eq!(mv("ss.s0", "inreview"), "exit=0");
    let shown = ferrule(p, &["card", "show", "ss.s0"]).text();
    assert!(shown.ends_with("2026-10-05T09:00 moved=inreview\nexit=0"));
    assert_eq!(mv("ss.s0", "done"), refused("inreview", "done", "commit"));
    ferrule(p, &["card", "log", "ss.s0", "commit=abc1234"]).text();
    assert_eq!(
        mv("ss.s0", "done"),
        refused("inreview", "done", "unverified")
    );
    ferrule(p, &["card", "log", "ss", "gauntlet=pass", "sha=abc1234def"]).text();
    assert_eq!(mv("ss.s0", "done"), "exit=0");
    assert_eq!(mv("ss.s0", "done"), "exit=0", "same state is a no-op");
    assert_eq!(mv("ss.s1", "inprogress"), "exit=0");
    assert_eq!(mv("ss.s2", "done"), refused("open", "done", "transition"));
    assert_eq!(
        mv("ss", "done"),
        refused("inprogress", "done", "transition")
    );

    new(p, &["tt"]);
    assert_eq!(
        mv("tt", "inprogress"),
        refused("open", "inprogress", "gate2")
    );
    assert_eq!(
        mv("tt", "inreview"),
        refused("open", "inreview", "transition")
    );
    new(p, &["ep", "kind=epic"]);
    assert_eq!(mv("ep", "inreview"), "exit=0");
    ferrule(p, &["card", "move", "nosuch.s1", "done"]).golden("card-move-no-card");
}

#[test]
fn it_new_and_set_validate_fields() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    new(p, &["r", "lane=fast"]);
    let run = |args: &[&str]| ferrule(p, args).text();
    assert_eq!(
        run(&["card", "new", "r"]),
        "card-new=exists hint=pick another id, or change the card with `ferrule card set`\nexit=1"
    );
    assert_eq!(
        run(&["card", "new", "r.s1", "lane=fast"]),
        "card-new=reject key=lane hint=that key is not a field of this kind of card; see `ferrule guide board`\nexit=2"
    );
    assert_eq!(
        run(&["card", "set", "r", "gate2=2026-10-03", "pr=7"]),
        "exit=0"
    );
    assert_eq!(run(&["card", "set", "r", "lane="]), "exit=0");
    assert_eq!(run(&["card", "field", "r", "gate2"]), "2026-10-03\nexit=0");
    assert_eq!(run(&["card", "field", "r", "lane"]), "exit=0");
    assert_eq!(run(&["card", "field", "r", "state"]), "open\nexit=0");
    assert_eq!(
        run(&["card", "set", "x", "pr=1"]),
        "card-set=card hint=check the id with `ferrule card board`\nexit=1"
    );
    assert_eq!(
        run(&["card", "show", "r"]),
        "---\nid: r\nstate: open\ngate2: 2026-10-03\npr: 7\n---\nexit=0"
    );
}

#[test]
fn it_import_is_all_or_nothing() {
    let p = project();
    let board = fixtures().join("board");
    let again = ferrule(p.path(), &["card", "import", board.to_str().unwrap()]).text();
    assert!(again.starts_with("import=refused reason=card "), "{again}");

    let bad = tempfile::tempdir().unwrap();
    std::fs::create_dir(bad.path().join("open")).unwrap();
    std::fs::write(bad.path().join("open/a.md"), "---\nid: a\n---\n").unwrap();
    std::fs::write(
        bad.path().join("open/b.md"),
        "---\nid: b\ncolor: red\n---\n",
    )
    .unwrap();
    let fresh = tempfile::tempdir().unwrap();
    let got = ferrule(
        fresh.path(),
        &["card", "import", bad.path().to_str().unwrap()],
    )
    .text();
    assert!(got.contains("unknown frontmatter key color"), "{got}");
    assert_eq!(ferrule(fresh.path(), &["card", "board"]).text(), "exit=0");
}

#[test]
fn it_board_lists_every_card_in_run_order() {
    let p = project();
    let got = ferrule(p.path(), &["card", "board"]).text();
    let first: Vec<&str> = got.lines().take(4).collect();
    assert_eq!(
        first,
        [
            "inprogress demo",
            "done       demo.s0  Land spec",
            "inreview   demo.s1  Add backoff",
            "open       demo.s2  Show status",
        ]
    );
}

#[test]
fn it_worktree_reaches_the_main_checkouts_board() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("r");
    std::fs::create_dir(&repo).unwrap();
    let git = |args: &[&str]| {
        let ok = std::process::Command::new("git")
            .arg("-C")
            .arg(&repo)
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
            .unwrap()
            .success();
        assert!(ok, "git {args:?}");
    };
    git(&["init", "-q", "-b", "main"]);
    git(&["commit", "-q", "--allow-empty", "-m", "init"]);
    let wt = dir.path().join("wt");
    git(&[
        "worktree",
        "add",
        "-q",
        wt.to_str().unwrap(),
        "-b",
        "feat/x",
    ]);
    new(&repo, &["x", "--state", "inprogress"]);
    let got = Run::new(["card", "log", "x", "note", "--", "from", "the", "worktree"])
        .env("FERRULE_NOW", NOW)
        .cwd(&wt)
        .text();
    assert_eq!(got, "exit=0");
    let shown = ferrule(&repo, &["card", "show", "x"]).text();
    assert!(shown.contains("note from the worktree"), "{shown}");
}

fn log_at(p: &Path, at: &str, args: &[&str]) {
    let mut all = vec!["card", "log"];
    all.extend_from_slice(args);
    let got = Run::new(&all)
        .env("CLAUDE_PROJECT_DIR", p)
        .env("FERRULE_NOW", at)
        .cwd(p)
        .text();
    assert_eq!(got, "exit=0", "{args:?}");
}

fn metrics(p: &Path, slug: &str) -> String {
    Run::new(["run", "metrics", slug])
        .env("CLAUDE_PROJECT_DIR", p)
        .env("FERRULE_NOW", "2026-10-04T12:00")
        .cwd(p)
        .text()
}

#[test]
fn it_metrics_count_the_runs_events_and_need_green_ci() {
    let dir = project();
    let p = dir.path();
    log_at(p, "2026-10-03T08:50", &["demo", "stop", "kind=approval"]);
    log_at(p, "2026-10-03T09:30", &["demo", "stop", "kind=question"]);
    log_at(p, "2026-10-03T10:40", &["demo", "replan", "from=s2"]);
    log_at(p, "2026-10-03T10:50", &["demo", "stop", "kind=approval"]);
    log_at(
        p,
        "2026-10-03T10:05",
        &["demo.s1", "test-red", "MB-R-161", "--", "it_backoff_caps"],
    );
    log_at(
        p,
        "2026-10-03T11:05",
        &[
            "demo",
            "overturned",
            "scope=stage",
            "count=2",
            "--",
            "restates",
        ],
    );
    log_at(
        p,
        "2026-10-03T11:06",
        &[
            "demo",
            "overturned",
            "scope=branch",
            "count=1",
            "--",
            "stale",
        ],
    );
    log_at(
        p,
        "2026-10-03T11:07",
        &[
            "demo.s1",
            "read",
            "path=ferrowl-modbus/src/client_core.rs",
            "--",
            "no",
            "signature",
        ],
    );
    log_at(p, "2026-10-04T11:50", &["demo", "ci=0"]);
    check("metrics", &metrics(p, "demo"));
    let all = || ferrule(p, &["run", "metrics"]).text();
    let first = all();
    assert_eq!(first.lines().count(), 2, "{first}");

    log_at(p, "2026-10-04T11:55", &["demo", "ci=1"]);
    let red = metrics(p, "demo");
    assert!(
        red.starts_with("metrics=refused reason=ci-red hint=") && red.ends_with("\nexit=1"),
        "{red}"
    );
    assert_eq!(all(), first, "a refused run records nothing");
    log_at(
        p,
        "2026-10-04T11:58",
        &["demo", "ci-waived", "--", "fails", "on", "main"],
    );
    assert!(metrics(p, "demo").ends_with(" ci=waived\nexit=0"));
    assert_eq!(all().lines().count(), 2, "a rerun replaces its line");
    assert!(metrics(p, "lb").ends_with(" lane=epic ci=-\nexit=0"));
    let none = metrics(p, "mrc");
    assert!(
        none.starts_with("metrics=refused reason=no-ci hint=") && none.ends_with("\nexit=1"),
        "{none}"
    );
    check("metrics-no-card", &metrics(p, "nosuch"));
}
