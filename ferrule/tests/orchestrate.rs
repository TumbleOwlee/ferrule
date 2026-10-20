//! The orchestration commands driving one run through its gates against a
//! real repository with a bare origin and a fake `gh`.

mod golden;

use std::path::{Path, PathBuf};
use std::process::Command;

use golden::{FakeGh, GH_FIELDS, Run, fixtures, gh_json, seed_log};

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

/// A clone of a bare origin holding main with one spec file, a passing
/// gauntlet and a fake `gh`.
struct Project {
    _dir: tempfile::TempDir,
    root: PathBuf,
    gh: FakeGh,
}

const PLAN: &str = "# demo — plan (rev 1)

## Shared
| Stage | Title | Files | Blocked by |
|---|---|---|---|
| s0 | Land spec | docs/specs/modbus/requirements.md | — |
| s1 | Add backoff | src/client.rs | s0 |
| s2 | Show status | src/tab.rs, src/view.rs | s1 |

## Stage s0: Land spec
";

impl Project {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let origin = dir.path().join("origin.git");
        let root = dir.path().join("r");
        git(
            dir.path(),
            &[
                "init",
                "-q",
                "--bare",
                "-b",
                "main",
                origin.to_str().unwrap(),
            ],
        );
        git(
            dir.path(),
            &[
                "clone",
                "-q",
                origin.to_str().unwrap(),
                root.to_str().unwrap(),
            ],
        );
        git(&root, &["checkout", "-q", "-b", "main"]);
        let specs = root.join("docs/specs/modbus");
        std::fs::create_dir_all(&specs).unwrap();
        std::fs::write(
            specs.join("requirements.md"),
            "**MB-R-001** — The client connects.\n",
        )
        .unwrap();
        std::fs::write(root.join(".gitignore"), "/.ferrule/\n").unwrap();
        git(&root, &["add", "."]);
        git(&root, &["commit", "-qm", "base"]);
        git(&root, &["push", "-q", "-u", "origin", "main"]);
        std::fs::create_dir_all(root.join(".ferrule")).unwrap();
        std::fs::write(
            root.join(".ferrule/ferrule.toml"),
            "[[gauntlet.step]]\nname = \"check\"\ncmd = [\"true\"]\n",
        )
        .unwrap();
        Project {
            _dir: dir,
            root,
            gh: FakeGh::new(),
        }
    }

    fn ferrule(&self, args: &[&str]) -> Run {
        self.gh
            .on(Run::new(args))
            .env("CLAUDE_PROJECT_DIR", &self.root)
            .env("FERRULE_NOW", "2026-10-05T09:00")
            .cwd(&self.root)
    }

    fn ok(&self, args: &[&str]) -> String {
        let got = self.ferrule(args).text();
        assert!(got.ends_with("exit=0"), "{args:?}: {got}");
        got.trim_end_matches("exit=0").trim_end().to_owned()
    }

    fn run_dir(&self) -> PathBuf {
        self.root.join(".ferrule/runs/demo")
    }

    fn worktree(&self) -> PathBuf {
        self.root.join(".ferrule/worktrees/demo")
    }

    fn field(&self, card: &str, key: &str) -> String {
        self.ok(&["board", "field", card, key])
    }

    /// The demo run's artifacts, the plan with its stage table.
    fn artifacts(&self) {
        let src = fixtures().join("board/artifacts/demo");
        for e in std::fs::read_dir(src).unwrap() {
            let e = e.unwrap();
            let name = e.file_name();
            if name != "request.md" {
                std::fs::copy(e.path(), self.run_dir().join(name)).unwrap();
            }
        }
        std::fs::write(self.run_dir().join("plan.md"), PLAN).unwrap();
    }

    /// Commits a change in the worktree as stage `sid` and logs it.
    fn land(&self, sid: &str, file: &str) -> String {
        let card = format!("demo.{sid}");
        self.ok(&["board", "move", &card, "inprogress"]);
        let path = self.worktree().join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, format!("{sid}\n")).unwrap();
        git(&self.worktree(), &["add", "."]);
        git(&self.worktree(), &["commit", "-qm", sid]);
        let sha = git(&self.worktree(), &["rev-parse", "--short", "HEAD"]);
        self.ok(&["board", "move", &card, "inreview"]);
        self.ok(&["board", "log", &card, &format!("commit={sha}")]);
        sha
    }
}

fn threads(path: &str) -> String {
    format!(
        r#"{{"data":{{"repository":{{"pullRequest":{{"state":"OPEN","isDraft":true,"reviewThreads":{{"nodes":[{{"id":"T1","isResolved":false,"path":"{path}","line":3,"originalLine":3,"comments":{{"nodes":[{{"databaseId":11,"author":{{"login":"al"}},"createdAt":"t","body":"why?"}}]}}}}]}}}}}}}}}}"#
    )
}

const NO_THREADS: &str = r#"{"data":{"repository":{"pullRequest":{"state":"OPEN","isDraft":true,"reviewThreads":{"nodes":[]}}}}}"#;

const THREADS_QUERY: &str = "query=query($owner: String!, $repo: String!, $pr: Int!) {\n  repository(owner: $owner, name: $repo) {";

fn graphql(pr: &str) -> Vec<String> {
    [
        "api",
        "graphql",
        "-F",
        "owner={owner}",
        "-F",
        "repo={repo}",
        "-F",
        &format!("pr={pr}"),
        "-f",
        THREADS_QUERY,
    ]
    .map(str::to_owned)
    .to_vec()
}

fn serve(gh: &FakeGh, args: &[String], text: &str) {
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    gh.serve(&args, text);
}

/// Init, the brief and the architecture approved, then gate 2.
fn through_gate2(p: &Project) {
    let gh = &p.gh;
    gh.serve(
        &["issue", "view", "7", "--json", GH_FIELDS],
        &gh_json(&std::fs::read_to_string(fixtures().join("issue/plain.txt")).unwrap()),
    );

    assert_eq!(
        p.ok(&["run", "init", "demo", "--issue", "7"]),
        "init=done slug=demo"
    );
    let request = std::fs::read_to_string(p.run_dir().join("request.md")).unwrap();
    assert!(
        request.starts_with("As a user I want my client to reconnect."),
        "{request}"
    );
    assert!(
        !p.run_dir().join("issue-spec.md").exists(),
        "a plain issue carries no brief"
    );
    assert!(
        p.ferrule(&["run", "init", "demo"])
            .text()
            .starts_with("init=refused reason=exists hint=")
    );
    p.artifacts();

    p.ok(&["prompt", "render", "demo", "gate1"]);
    let gate1 = p.ok(&["gate", "approve", "demo", "gate1", "--lane", "full"]);
    assert!(
        gate1.starts_with("approve=done gate=gate1 spec-rev=5"),
        "{gate1}"
    );
    assert_eq!(p.field("demo", "gate1"), "2026-10-05");
    assert_eq!(p.field("demo", "lane"), "full");
    p.ok(&["gate", "approve", "demo", "arch"]);

    gh.serve(
        &[
            "issue",
            "list",
            "--state",
            "open",
            "--search",
            "Reconnect dropped Modbus clients in:title",
            "--json",
            "number,title",
        ],
        "[]",
    );
    gh.serve(
        &[
            "issue",
            "create",
            "--title",
            "Reconnect dropped Modbus clients",
            "--body-file",
            "-",
        ],
        "https://github.com/o/r/issues/12\n",
    );
    let gate2 = p.ok(&["gate", "approve", "demo", "gate2", "--issue", "new"]);
    assert!(
        gate2.starts_with(
            "approve=done gate=gate2 issue=12 worktree=.ferrule/worktrees/demo stages=3"
        ),
        "{gate2}"
    );
    assert_eq!(p.field("demo", "issue"), "12");
    assert_eq!(p.field("demo", "branch"), "feat/demo");
    assert_eq!(p.field("demo", "state"), "inprogress");
    assert_eq!(p.field("demo.s1", "blocked-by"), "[demo.s0]");
    assert_eq!(p.field("demo.s2", "files"), "[src/tab.rs, src/view.rs]");
    assert_eq!(p.field("demo.s1", "stop"), "yes");
    assert_eq!(p.field("demo.s2", "stop"), "no");
    assert!(p.worktree().join("docs").is_dir());
    assert!(
        gh.log()
            .contains("issue create --title Reconnect dropped Modbus clients --body-file -")
    );
}

/// s0 lands: the first push opens the draft PR, a move re-renders it, a
/// review thread sends s0 back.
fn first_stage(p: &Project) {
    let gh = &p.gh;
    let sha0 = p.land("s0", "docs/specs/modbus/requirements.md");
    gh.serve(
        &[
            "pr",
            "create",
            "--draft",
            "--head",
            "feat/demo",
            "--base",
            "main",
            "--title",
            "Reconnect dropped Modbus clients",
            "--body-file",
            "-",
        ],
        "https://github.com/o/r/pull/34\n",
    );
    let pushed = p.ok(&["pr", "push", "demo"]);
    assert!(pushed.starts_with("push=done sha="), "{pushed}");
    assert!(
        pushed.ends_with(" gauntlet=pass pr-body=opened pr=34"),
        "{pushed}"
    );
    assert_eq!(p.field("demo", "pr"), "34");
    assert_eq!(
        git(&p.root, &["rev-parse", "--short", "origin/feat/demo"]),
        sha0
    );
    assert!(p.field("demo", "pass-sha").starts_with(&sha0));

    gh.serve(
        &["pr", "view", "34", "--json", "state,isDraft,body"],
        r#"{"state":"OPEN","isDraft":true,"body":"stale"}"#,
    );
    gh.serve(&["pr", "edit", "34", "--body-file", "-"], "");
    assert_eq!(
        p.ok(&["board", "move", "demo.s0", "done"]),
        "pr-body=pushed"
    );
    assert!(
        gh.log().contains("pr edit 34 --body-file -"),
        "{}",
        gh.log()
    );

    serve(
        gh,
        &graphql("34"),
        &threads("docs/specs/modbus/requirements.md"),
    );
    assert_eq!(
        p.ok(&["pr", "feedback", "route", "demo"]),
        "pr=34 threads=1 stages=s0\npr-body=pushed"
    );
    assert_eq!(p.field("demo.s0", "state"), "inprogress");
    assert!(p.run_dir().join("pr-feedback.md").is_file());
}

/// Gate 4 published, then CI red and green.
fn publish_and_ci(p: &Project) {
    let gh = &p.gh;
    p.ok(&["board", "move", "demo.s0", "inreview"]);
    p.ok(&["board", "move", "demo.s0", "done"]);
    serve(gh, &graphql("34"), NO_THREADS);
    gh.serve(&["pr", "ready", "34"], "");
    gh.serve(
        &[
            "pr",
            "edit",
            "34",
            "--body-file",
            "-",
            "--title",
            "Reconnect dropped Modbus clients",
        ],
        "",
    );
    gh.serve(
        &[
            "issue",
            "list",
            "--state",
            "open",
            "--search",
            "Share the backoff helper with the RTU client in:title",
            "--json",
            "number,title",
        ],
        r#"[{"number":40,"title":"Share the backoff helper with the RTU client"}]"#,
    );
    let published = p.ok(&["pr", "publish", "demo"]);
    assert_eq!(published, "publish=done pr=34 follow-ups=#40");
    assert!(
        gh.log()
            .contains("pr edit 34 --body-file - --title Reconnect dropped Modbus clients")
    );
    let shown = p.ok(&["board", "show", "demo"]);
    assert!(
        shown.contains("pr=ready") && shown.contains("follow-ups=[40]"),
        "{shown}"
    );

    gh.serve_exit(
        &["pr", "checks", "34", "--watch"],
        "test\tfail\t1m\thttps://x\n",
        1,
    );
    gh.serve(
        &[
            "run",
            "list",
            "--branch",
            "feat/demo",
            "--status",
            "failure",
            "--limit",
            "1",
            "--json",
            "databaseId",
        ],
        "[]",
    );
    let red = p.ok(&["ci", "wait", "demo"]);
    assert!(red.starts_with("ci=1 failed=test\n"), "{red}");
    gh.serve(
        &["pr", "checks", "34", "--watch"],
        "test\tpass\t1m\thttps://x\n",
    );
    std::fs::remove_file(gh.dir.path().join("fx").join("pr_checks_34_--watch.exit")).unwrap();
    let green = p.ok(&["ci", "wait", "demo"]);
    assert!(green.starts_with("ci=0\n"), "{green}");
    assert!(green.contains("ferrule run cleanup demo"), "{green}");
}

#[test]
fn it_one_run_from_init_to_ci() {
    let p = Project::new();
    through_gate2(&p);
    first_stage(&p);
    publish_and_ci(&p);
}

#[test]
fn it_publish_refuses_threads_and_stage_refs() {
    let p = Project::new();
    p.ok(&["run", "init", "demo"]);
    p.artifacts();
    p.ok(&["board", "set", "demo", "pr=34", "branch=feat/demo"]);
    serve(&p.gh, &graphql("34"), &threads("src/x.rs"));
    assert_eq!(
        p.ferrule(&["pr", "publish", "demo"]).text(),
        "publish=refused reason=threads threads=1 \
         hint=resolve the open threads: ferrule guide show feedback\nexit=1"
    );
    serve(&p.gh, &graphql("34"), NO_THREADS);
    assert_eq!(
        p.ferrule(&["pr", "publish", "demo"]).text(),
        "publish=refused reason=collisions-unknown hint=main could not be fetched \
         or shares no history with the branch; ask the user to fix the remote, \
         then publish again\nexit=1"
    );
    git(
        &p.root,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "feat/demo",
            p.worktree().to_str().unwrap(),
        ],
    );
    serve(&p.gh, &graphql("34"), NO_THREADS);
    let pr = p.run_dir().join("pr.md");
    let text = std::fs::read_to_string(&pr)
        .unwrap()
        .replace("retry with backoff", "retry as in s2");
    std::fs::write(&pr, text).unwrap();
    assert_eq!(
        p.ferrule(&["pr", "publish", "demo"]).text(),
        "publish=refused reason=stage-ref stage-ref=s2 \
         hint=reword the text naming it, then publish again\nexit=3"
    );
}

#[test]
fn it_push_refuses_an_unverified_head_and_main() {
    let p = Project::new();
    p.ok(&["run", "init", "demo"]);
    p.ok(&["board", "set", "demo", "branch=main"]);
    assert_eq!(
        p.ferrule(&["pr", "push", "demo"]).text(),
        "push=refused reason=main hint=push from a branch other than main\nexit=1"
    );
    p.ok(&["board", "set", "demo", "branch=feat/demo"]);
    git(
        &p.root,
        &[
            "worktree",
            "add",
            "-q",
            p.worktree().to_str().unwrap(),
            "-b",
            "feat/demo",
        ],
    );
    std::fs::write(p.worktree().join("wip.txt"), "wip\n").unwrap();
    assert_eq!(
        p.ferrule(&["pr", "push", "demo"]).text(),
        "push=refused reason=dirty hint=commit the changes through `ferrule stage commit`, \
         then push again\nexit=1"
    );
    std::fs::remove_file(p.worktree().join("wip.txt")).unwrap();
    assert_eq!(
        p.ferrule(&["pr", "push", "demo"]).text(),
        "push=refused reason=sha mismatch=card-commit hint=push a stage's last commit= \
         on the run's branch: ferrule stage verify\nexit=1"
    );
}

#[test]
fn it_replan_swaps_the_uncommitted_stages() {
    let p = Project::new();
    p.ok(&["run", "init", "demo"]);
    p.artifacts();
    p.gh.serve(
        &[
            "issue",
            "list",
            "--state",
            "open",
            "--search",
            "Reconnect dropped Modbus clients in:title",
            "--json",
            "number,title",
        ],
        r#"[{"number":12,"title":"Reconnect dropped Modbus clients"}]"#,
    );
    let gate2 = p.ok(&["gate", "approve", "demo", "gate2"]);
    assert!(gate2.contains("issue=12 "), "adopted: {gate2}");
    let sha0 = p.land("s0", "docs/specs/modbus/requirements.md");
    seed_log(
        &p.root,
        "2026-10-05T09:00",
        "demo",
        &["gauntlet=pass", "cov=80%", &format!("sha={sha0}")],
    );
    p.ok(&["board", "move", "demo.s0", "done"]);
    p.ok(&["board", "move", "demo.s1", "inprogress"]);
    p.ok(&[
        "board",
        "log",
        "demo.s1",
        "spec-delta",
        "MB-R-170",
        "added",
        "origin=impl",
        "--",
        "a new one",
    ]);

    assert_eq!(
        p.ok(&["stage", "replan", "demo", "start"]),
        "replan=started from=s1"
    );
    assert!(p.run_dir().join("plan.prev.md").is_file());
    std::fs::write(
        p.run_dir().join("plan.md"),
        PLAN.replace("| s1 | Add backoff | src/client.rs | s0 |\n| s2 | Show status | src/tab.rs, src/view.rs | s1 |", "| s1 | Retry loop | src/retry.rs | s0 |"),
    )
    .unwrap();
    assert_eq!(
        p.ok(&["stage", "replan", "demo", "approve", "--stops", "s1"]),
        "replan=approved from=s1 stages=1"
    );
    assert_eq!(p.field("demo.s1", "title"), "Retry loop");
    assert_eq!(p.field("demo.s1", "stop"), "yes");
    assert_eq!(
        p.ferrule(&["board", "show", "demo.s2"]).text(),
        "card-show=card hint=check the id with `ferrule board list`\nexit=1"
    );
    let parent = p.ok(&["board", "show", "demo"]);
    assert!(
        parent.contains("spec-delta MB-R-170 added origin=impl retired"),
        "{parent}"
    );
    assert!(parent.contains("replan=approved"), "{parent}");
}

#[test]
fn it_replan_approve_is_refused_twice() {
    let p = Project::new();
    p.ok(&["run", "init", "demo"]);
    p.artifacts();
    p.gh.serve(
        &[
            "issue",
            "list",
            "--state",
            "open",
            "--search",
            "Reconnect dropped Modbus clients in:title",
            "--json",
            "number,title",
        ],
        r#"[{"number":12,"title":"Reconnect dropped Modbus clients"}]"#,
    );
    p.ok(&["gate", "approve", "demo", "gate2"]);
    let sha0 = p.land("s0", "docs/specs/modbus/requirements.md");
    seed_log(
        &p.root,
        "2026-10-05T09:00",
        "demo",
        &["gauntlet=pass", "cov=80%", &format!("sha={sha0}")],
    );
    p.ok(&["board", "move", "demo.s0", "done"]);
    p.ok(&["stage", "replan", "demo", "start"]);
    p.ok(&["stage", "replan", "demo", "approve"]);
    let sha = p.land("s1", "src/client.rs");
    let refused = p.ferrule(&["stage", "replan", "demo", "approve"]).text();
    assert!(
        refused.starts_with("replan=refused reason=no-replan hint="),
        "{refused}"
    );
    assert_eq!(p.field("demo.s1", "state"), "inreview");
    let s1 = p.ok(&["board", "show", "demo.s1"]);
    assert!(s1.contains(&format!("commit={sha}")), "{s1}");
}

#[test]
fn it_init_a_slice_stores_its_epic() {
    let p = Project::new();
    p.gh.serve(
        &["issue", "view", "71", "--json", GH_FIELDS],
        &gh_json(&std::fs::read_to_string(fixtures().join("issue/slice.txt")).unwrap()),
    );
    let epic = "#70 Load balancing core [OPEN]\nhttps://github.com/o/r/issues/70\nby someone\n\n## Spec (rev 1)\n\n### Scenarios\n- **S1** — x\n\n--- 0 comment(s) ---\n";
    p.gh.serve(
        &["issue", "view", "70", "--json", GH_FIELDS],
        &gh_json(epic),
    );
    assert_eq!(
        p.ok(&["run", "init", "sl", "--issue", "71"]),
        "init=done slug=sl kind=slice epic=70"
    );
    assert_eq!(p.field("sl", "kind"), "slice");
    let dir = p.root.join(".ferrule/runs/sl");
    assert_eq!(
        std::fs::read_to_string(dir.join("slice-line.md")).unwrap(),
        "**Part of epic** #70 · **Slice** 1 of 3 · **Depends on** none\n"
    );
    let epic_md = std::fs::read_to_string(dir.join("epic.md")).unwrap();
    assert!(epic_md.starts_with("## Spec (rev 1)\n"), "{epic_md}");
}

#[test]
fn it_issue_file_variants() {
    let p = Project::new();
    let gh = &p.gh;
    p.ok(&["run", "init", "demo"]);
    p.artifacts();
    assert_eq!(
        p.ferrule(&["issue", "post", "demo", "--split"]).text(),
        "post=refused reason=no-issue \
         hint=file the tracking issue first: ferrule issue post <slug>\nexit=1"
    );
    p.ok(&["board", "set", "demo", "issue=12"]);
    assert_eq!(p.ok(&["issue", "post", "demo"]), "issue=12 action=recorded");
    gh.serve(
        &[
            "issue",
            "list",
            "--state",
            "open",
            "--search",
            "RTU clients reconnect in:title",
            "--json",
            "number,title",
        ],
        "[]",
    );
    gh.serve(
        &[
            "issue",
            "create",
            "--title",
            "RTU clients reconnect",
            "--body-file",
            "-",
        ],
        "https://github.com/o/r/issues/56\n",
    );
    assert_eq!(
        p.ok(&["issue", "post", "demo", "--split"]),
        "issue=56 action=created"
    );
    assert_eq!(p.field("demo", "split"), "[56]");
    std::fs::write(
        p.run_dir().join("issue-comment.md"),
        "MB-R-161 now retries forever.\n",
    )
    .unwrap();
    gh.serve(&["issue", "comment", "12", "--body-file", "-"], "");
    assert_eq!(
        p.ok(&["issue", "post", "demo", "--comment"]),
        "issue=12 action=commented"
    );
    assert!(gh.log().contains("issue comment 12 --body-file -"));
    gh.serve(
        &[
            "issue",
            "edit",
            "41",
            "--title",
            "Reconnect dropped Modbus clients",
            "--body-file",
            "-",
        ],
        "",
    );
    assert_eq!(
        p.ok(&["issue", "post", "demo", "--reuse", "41"]),
        "issue=41 action=edited"
    );
    assert_eq!(p.field("demo", "issue"), "41");
    assert_eq!(
        p.ferrule(&["issue", "post", "demo", "--split", "--comment"])
            .text(),
        "exit=2"
    );
}

#[test]
fn it_reconcile_checks_the_host() {
    let p = Project::new();
    let gh = &p.gh;
    p.ok(&["run", "init", "demo"]);
    p.artifacts();
    p.ok(&["board", "set", "demo", "issue=12", "pr=34"]);
    seed_log(&p.root, "2026-10-05T09:00", "demo", &["pr=ready"]);
    let closed = "#12 Reconnect dropped Modbus clients [CLOSED]\nhttps://github.com/o/r/issues/12\nby someone\n\nbody\n\n--- 0 comment(s) ---\n";
    gh.serve(
        &["issue", "view", "12", "--json", GH_FIELDS],
        &gh_json(closed),
    );
    gh.serve(
        &["pr", "view", "34", "--json", "state,isDraft,body"],
        r#"{"state":"OPEN","isDraft":true,"body":"x"}"#,
    );
    let out = p.ferrule(&["run", "reconcile", "demo"]).text();
    assert!(out.contains("differ row=issue #12 is not open\n"), "{out}");
    assert!(
        out.contains("differ row=pr #34 is a draft but pr=ready is logged\n"),
        "{out}"
    );
    assert!(out.ends_with("exit=1"), "{out}");
    let local = p.ferrule(&["run", "reconcile", "demo", "--local"]).text();
    assert!(!local.contains("issue #12"), "{local}");
}

#[test]
fn it_card_commit_checks_the_card_and_the_branch() {
    let p = Project::new();
    p.ok(&["run", "init", "demo"]);
    p.ok(&["board", "set", "demo", "branch=feat/demo"]);
    p.ok(&["board", "new", "demo.s1", "title=Add a"]);
    git(
        &p.root,
        &[
            "worktree",
            "add",
            "-q",
            p.worktree().to_str().unwrap(),
            "-b",
            "feat/demo",
        ],
    );
    std::fs::write(p.worktree().join("a.txt"), "a\n").unwrap();
    git(&p.worktree(), &["add", "a.txt"]);

    let in_wt = |args: &[&str]| {
        p.gh.on(Run::new(args))
            .env("CLAUDE_PROJECT_DIR", &p.root)
            .env("FERRULE_NOW", "2026-10-05T09:00")
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .cwd(p.worktree())
            .text()
    };
    assert_eq!(
        in_wt(&["stage", "commit", "demo.s1", "-m", "Add a"]),
        "commit=refused reason=state state=open hint=move the card to inreview first: `ferrule board move demo.s1 inreview`\nexit=1"
    );
    p.ok(&["board", "move", "demo.s1", "inprogress"]);
    p.ok(&["board", "move", "demo.s1", "inreview"]);
    let trailer = format!("{}: A <a@b>", ["Co", "Authored", "By"].join("-"));
    let done = in_wt(&["stage", "commit", "demo.s1", "-m", "Add a", "-m", &trailer]);
    let sha = git(&p.worktree(), &["rev-parse", "--short", "HEAD"]);
    assert_eq!(done, format!("commit=done sha={sha} card=demo.s1\nexit=0"));
    assert_eq!(git(&p.worktree(), &["log", "-1", "--format=%B"]), "Add a");
    assert_eq!(p.field("demo.s1", "log:commit"), sha);

    let on_main = p
        .ferrule(&["stage", "commit", "--no-run", "-m", "x"])
        .text();
    assert_eq!(
        on_main,
        "commit=refused reason=main branch=main hint=commit on the run's branch, not main: `ferrule gate approve <slug> gate2` creates it and its worktree; with --no-run, check out another branch first\nexit=1"
    );
}
