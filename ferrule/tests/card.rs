mod golden;

use std::path::Path;

use golden::{Run, check, fixtures, seed_log};

const NOW: &str = "2026-10-05T09:00";

/// A project whose state database holds the fixture board.
fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let board = fixtures().join("board");
    let got = ferrule(dir.path(), &["board", "import", board.to_str().unwrap()]).text();
    assert_eq!(got, "import=ok cards=13\nexit=0");
    dir
}

fn ferrule(project: &Path, args: &[&str]) -> Run {
    Run::new(args)
        .env("CLAUDE_PROJECT_DIR", project)
        .env("FERRULE_NOW", NOW)
        .cwd(project)
}

/// `board log`, then the card's last log line when it was accepted, as the
/// workflow's tests read it back with `tail -1`.
fn log_tail(project: &Path, card: &str, args: &[&str]) -> String {
    let mut all = vec!["board", "log", card];
    all.extend_from_slice(args);
    let out = ferrule(project, &all).text();
    if out != "exit=0" {
        return out;
    }
    let shown = ferrule(project, &["board", "show", card]).text();
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
        let out = ferrule(p.path(), &["board", "field", "demo", f]).output();
        got.push_str(f);
        got.push('=');
        got.push_str(&String::from_utf8_lossy(&out.stdout));
        got.push('\n');
    }
    let none = ferrule(p.path(), &["board", "field", "nosuch", "lane"]).text();
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
            &[
                "overturned",
                "scope=branch",
                "count=1",
                "--",
                "wrong",
                "finding",
            ],
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
    let before = ferrule(p, &["board", "show", "demo.s1"]).text();
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
        (
            "card-log-reject-owner",
            &["demo", "gauntlet=pass", "sha=abc1234"],
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
        let mut all = vec!["board", "log"];
        all.extend_from_slice(args);
        ferrule(p, &all).golden(name);
    }
    assert_eq!(ferrule(p, &["board", "show", "demo.s1"]).text(), before);
    let waived = ferrule(p, &["board", "log", "demo", "ci-waived"]).output();
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
            &[
                "board", "log", "demo.s2", "read", &path, "--", "the", "plan",
            ],
        )
        .text();
        assert_eq!(got, "exit=0");
    }
    ferrule(
        p,
        &[
            "board",
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
    ferrule(p, &["board", "log", "demo.s3", "commit=abc1234"])
        .golden("card-log-reject-commit-inprogress");
    let shown = ferrule(p, &["board", "show", "demo.s3"]).text();
    assert!(!shown.contains("commit="));
    assert_eq!(
        log_tail(p, "demo.s1", &["commit=abc1234"]),
        "2026-10-05T09:00 commit=abc1234\nexit=0"
    );
}

fn new(p: &Path, args: &[&str]) {
    let mut all = vec!["board", "new"];
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
    let mv = |card: &str, to: &str| ferrule(p, &["board", "move", card, to]).text();
    let refused = |from: &str, to: &str, reason: &str| {
        let hint = match reason {
            "transition" => "TRANSITION",
            "blocked-by" => "move the cards in its blocked-by to done first",
            "commit" => "commit the stage with `ferrule stage commit <card> -m <msg>` first",
            "unverified" => {
                "run `ferrule ci gauntlet <slug>`, then `ferrule pr push <slug>`, which logs the gauntlet=pass that covers the commit sha"
            }
            _ => "approve gate 2 first: `ferrule gate approve <slug> gate2`",
        };
        format!("card-move=refused reason={reason} from={from} to={to} hint={hint}\nexit=2")
    };
    let moves = |line: String, hint: &str| line.replace("TRANSITION", hint);

    assert_eq!(
        mv("ss.s1", "inprogress"),
        refused("open", "inprogress", "blocked-by")
    );
    assert_eq!(mv("ss.s0", "inprogress"), "exit=0");
    assert_eq!(mv("ss.s0", "inreview"), "exit=0");
    let shown = ferrule(p, &["board", "show", "ss.s0"]).text();
    assert!(shown.ends_with("2026-10-05T09:00 moved=inreview\nexit=0"));
    assert_eq!(mv("ss.s0", "done"), refused("inreview", "done", "commit"));
    ferrule(p, &["board", "log", "ss.s0", "commit=abc1234"]).text();
    assert_eq!(
        mv("ss.s0", "done"),
        refused("inreview", "done", "unverified")
    );
    seed_log(p, NOW, "ss", &["gauntlet=pass", "sha=abc1234def"]);
    assert_eq!(mv("ss.s0", "done"), "exit=0");
    assert_eq!(mv("ss.s0", "done"), "exit=0", "same state is a no-op");
    assert_eq!(mv("ss.s1", "inprogress"), "exit=0");
    assert_eq!(
        mv("ss.s2", "done"),
        moves(
            refused("open", "done", "transition"),
            "from open a stage card can move only to inprogress"
        )
    );
    assert_eq!(
        mv("ss", "done"),
        moves(
            refused("inprogress", "done", "transition"),
            "from inprogress a parent card can move only to inreview"
        )
    );

    new(p, &["tt"]);
    assert_eq!(
        mv("tt", "inprogress"),
        refused("open", "inprogress", "gate2")
    );
    assert_eq!(
        mv("tt", "inreview"),
        moves(
            refused("open", "inreview", "transition"),
            "from open a parent card can move only to inprogress (after gate2), or inreview for an epic"
        )
    );
    new(p, &["ep", "kind=epic"]);
    assert_eq!(mv("ep", "inreview"), "exit=0");
    ferrule(p, &["board", "move", "nosuch.s1", "done"]).golden("card-move-no-card");
}

#[test]
fn it_new_and_set_validate_fields() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    new(p, &["r", "lane=fast"]);
    let run = |args: &[&str]| ferrule(p, args).text();
    assert_eq!(
        run(&["board", "new", "r"]),
        "card-new=refused reason=exists hint=pick another id, or change the card with `ferrule board set`\nexit=1"
    );
    assert_eq!(
        run(&["board", "new", "r.s1", "lane=fast"]),
        "card-new=reject reason=key key=lane hint=a stage card takes only title, blocked-by, files, branch, worktree, stop\nexit=2"
    );
    assert_eq!(
        run(&["board", "set", "r", "gate2=2026-10-03", "pr=7"]),
        "exit=0"
    );
    assert_eq!(
        run(&["board", "set", "r", "issue=9", "color=red"]),
        "card-set=reject reason=key key=color hint=a parent card takes only kind, epic, slices, issue, pr, branch, lane, gate1, with-arch, with-plan, skipped, arch, no-arch, gate2, gate3, gate4, split, upstream, artifacts, worktree\nexit=2"
    );
    assert_eq!(run(&["board", "field", "r", "issue"]), "exit=0");
    assert_eq!(run(&["board", "set", "r", "lane="]), "exit=0");
    assert_eq!(run(&["board", "field", "r", "gate2"]), "2026-10-03\nexit=0");
    assert_eq!(run(&["board", "field", "r", "lane"]), "exit=0");
    assert_eq!(run(&["board", "field", "r", "state"]), "open\nexit=0");
    assert_eq!(
        run(&["board", "set", "x", "pr=1"]),
        "card-set=refused reason=card hint=check the slug with `ferrule board list`\nexit=1"
    );
    assert_eq!(
        run(&["board", "show", "r"]),
        "---\nid: r\nstate: open\ngate2: 2026-10-03\npr: 7\n---\nexit=0"
    );
}

#[test]
fn it_import_is_all_or_nothing() {
    let p = project();
    let board = fixtures().join("board");
    let again = ferrule(p.path(), &["board", "import", board.to_str().unwrap()]).text();
    assert!(
        again.starts_with("import=refused reason=invalid detail=\"card "),
        "{again}"
    );

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
        &["board", "import", bad.path().to_str().unwrap()],
    )
    .text();
    assert!(got.contains("unknown frontmatter key color"), "{got}");
    assert_eq!(ferrule(fresh.path(), &["board", "list"]).text(), "exit=0");

    let p = project();
    let out = tempfile::tempdir().unwrap();
    let show = ferrule(p.path(), &["board", "show", "demo.s2"]).text();
    let show = show.trim_end_matches("exit=0");
    std::fs::create_dir(out.path().join("open")).unwrap();
    std::fs::write(out.path().join("open/demo.s2.md"), show).unwrap();
    let fresh = tempfile::tempdir().unwrap();
    let got = ferrule(
        fresh.path(),
        &["board", "import", out.path().to_str().unwrap()],
    )
    .text();
    assert_eq!(got, "import=ok cards=1\nexit=0");
    std::fs::write(
        out.path().join("open/demo.s2.md"),
        show.replace("state: open", "state: done"),
    )
    .unwrap();
    let got = ferrule(
        tempfile::tempdir().unwrap().path(),
        &["board", "import", out.path().to_str().unwrap()],
    )
    .text();
    assert!(got.contains("differs from the open directory"), "{got}");
}

#[test]
fn it_board_lists_every_card_in_run_order() {
    let p = project();
    let got = ferrule(p.path(), &["board", "list"]).text();
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
fn it_board_list_filters_by_state_and_run() {
    let p = project();
    let got = ferrule(
        p.path(),
        &["board", "list", "--run", "demo", "--state", "open"],
    )
    .text();
    assert_eq!(got, "open       demo.s2  Show status\nexit=0");
    let ids: Vec<String> = ferrule(p.path(), &["board", "list", "--run", "demo"])
        .text()
        .lines()
        .filter_map(|l| l.split_whitespace().nth(1).map(str::to_owned))
        .collect();
    assert!(
        ids.iter().all(|id| id == "demo" || id.starts_with("demo.")),
        "{ids:?}"
    );
    assert!(ids.len() >= 4, "{ids:?}");

    let dotted = tempfile::tempdir().unwrap();
    let d = dotted.path();
    for id in ["my", "my.slug", "my.slug.s1", "my.s1"] {
        new(d, &[id]);
    }
    let list = ferrule(d, &["board", "list", "--run", "my"]).text();
    assert_eq!(list.lines().count(), 3, "{list}");
    assert!(!list.contains("my.slug"), "{list}");
    let list = ferrule(d, &["board", "list", "--run", "my.slug"]).text();
    assert_eq!(list.lines().count(), 3, "{list}");
}

#[test]
fn it_board_show_filters_fields_and_log() {
    let p = project();
    let show = |args: &[&str]| {
        let mut all = vec!["board", "show", "demo"];
        all.extend(args);
        ferrule(p.path(), &all).text()
    };
    let fields = show(&["--fields"]);
    assert!(fields.starts_with("---\nid: demo\n"), "{fields}");
    assert!(fields.ends_with("---\nexit=0"), "{fields}");
    assert_eq!(
        show(&["--log", "--event", "gauntlet"]),
        "2026-10-03T10:00 gauntlet=pass cov=80.1% sha=aaa1111\n2026-10-03T11:00 gauntlet=pass cov=81.2% sha=bbb2222\nexit=0"
    );
    assert_eq!(
        show(&["--log", "--event", "gauntlet", "--last", "1"]),
        "2026-10-03T11:00 gauntlet=pass cov=81.2% sha=bbb2222\nexit=0"
    );
    let with_fields = show(&["--last", "1"]);
    assert!(with_fields.starts_with("---\nid: demo\n"), "{with_fields}");
    assert_eq!(
        with_fields
            .lines()
            .filter(|l| l.starts_with("2026-"))
            .count(),
        1
    );
}

#[test]
fn it_board_vocab_filters_by_event_and_kind() {
    let p = project();
    let one = ferrule(p.path(), &["board", "vocab", "--event", "test-red"]).text();
    assert_eq!(one.lines().count(), 3, "{one}");
    assert!(
        one.lines().nth(1).unwrap().starts_with("stage   test-red"),
        "{one}"
    );
    let stage = ferrule(p.path(), &["board", "vocab", "--kind", "stage"]).text();
    assert!(
        !stage.lines().skip(1).any(|l| l.starts_with("parent")),
        "{stage}"
    );
    assert!(stage.lines().any(|l| l.starts_with("both")), "{stage}");
    assert_eq!(
        ferrule(
            p.path(),
            &["board", "vocab", "--event", "ci-waived", "--kind", "stage"]
        )
        .text(),
        "vocab=none hint=no such event for this card kind; ferrule board vocab lists them all\nexit=1"
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
    let got = Run::new(["board", "log", "x", "note", "--", "from", "the", "worktree"])
        .env("FERRULE_NOW", NOW)
        .cwd(&wt)
        .text();
    assert_eq!(got, "exit=0");
    let shown = ferrule(&repo, &["board", "show", "x"]).text();
    assert!(shown.contains("note from the worktree"), "{shown}");
}

fn log_at(p: &Path, at: &str, args: &[&str]) {
    seed_log(p, at, args[0], &args[1..]);
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

#[test]
fn it_metrics_last_takes_no_slug() {
    let p = project();
    assert_eq!(
        ferrule(p.path(), &["run", "metrics", "--last", "2"]).text(),
        "exit=0"
    );
    assert_eq!(
        ferrule(p.path(), &["run", "metrics", "demo", "--last", "2"]).text(),
        "exit=2"
    );
}

#[test]
fn it_log_refuses_events_only_ferrule_writes() {
    let p = project();
    assert_eq!(
        log_tail(p.path(), "demo", &["gauntlet=pass", "sha=x"]),
        "card-log=reject reason=owner event=gauntlet hint=only ferrule logs it, with `ferrule pr push <slug>`\nexit=2"
    );
    let refused = log_tail(p.path(), "demo", &["ci=0"]);
    assert!(refused.contains("reason=owner"), "{refused}");
    // The events the guides have agents log stay open to them.
    for (card, args) in [
        ("demo.s1", vec!["checks=pass", "crates=a"]),
        ("demo.s1", vec!["commit=abc"]),
        ("demo", vec!["spec-keep=MB-R-001"]),
        ("demo", vec!["ci-waived", "--", "red on main too"]),
    ] {
        let got = log_tail(p.path(), card, &args);
        assert!(got.ends_with("exit=0"), "{args:?}: {got}");
    }
    let vocab = ferrule(p.path(), &["board", "vocab", "--event", "gauntlet"]).text();
    assert!(vocab.contains("owner"), "{vocab}");
    assert!(vocab.contains("ferrule"), "{vocab}");
}

#[test]
fn it_bad_now_is_refused() {
    let p = project();
    let got = Run::new(["board", "log", "demo", "note", "--", "x"])
        .env("CLAUDE_PROJECT_DIR", p.path())
        .env("FERRULE_NOW", "yesterday")
        .cwd(p.path())
        .text();
    assert_eq!(got, "exit=2");
    let out = Run::new(["board", "list"])
        .env("CLAUDE_PROJECT_DIR", p.path())
        .env("FERRULE_NOW", "2026-10-05")
        .cwd(p.path())
        .output();
    assert_eq!(out.status.code(), Some(2));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("ferrule=refused reason=now hint=FERRULE_NOW=2026-10-05"),
        "{err}"
    );
}

#[test]
fn it_import_refuses_a_bad_stamp() {
    let dir = tempfile::tempdir().unwrap();
    let board = tempfile::tempdir().unwrap();
    std::fs::create_dir(board.path().join("open")).unwrap();
    std::fs::write(
        board.path().join("open/x.md"),
        "---\nid: x\nstate: open\n---\nyesterday note -- hi\n",
    )
    .unwrap();
    let got = ferrule(
        dir.path(),
        &["board", "import", board.path().to_str().unwrap()],
    )
    .text();
    assert!(
        got.contains("import=refused reason=")
            && got.contains("log stamp yesterday is not a stamp"),
        "{got}"
    );
    assert!(got.ends_with("exit=2"), "{got}");
}
