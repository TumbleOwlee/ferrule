//! The orchestration commands driving one run through its gates against a
//! real repository with a bare origin and a fake `gh`.

mod golden;

use std::path::{Path, PathBuf};
use std::process::Command;

use golden::{FakeGh, GH_FIELDS, Run, fixtures, gh_json, seed_card, seed_log, seed_set};

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

    /// `ferrule` run inside the run's worktree, with a git identity.
    fn in_wt(&self, args: &[&str]) -> String {
        self.gh
            .on(Run::new(args))
            .env("CLAUDE_PROJECT_DIR", &self.root)
            .env("FERRULE_NOW", "2026-10-05T09:00")
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .cwd(self.worktree())
            .text()
    }

    /// `ferrule` run with the clock at `now`.
    fn at(&self, now: &str, args: &[&str]) -> String {
        self.ferrule(args).env("FERRULE_NOW", now).text()
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

    /// The bare origin's path.
    fn origin(&self) -> PathBuf {
        self.root.parent().unwrap().join("origin.git")
    }

    /// A sibling clone of the origin on `branch`.
    fn sibling(&self, name: &str, branch: &str) -> PathBuf {
        let dir = self.root.parent().unwrap().join(name);
        git(
            self.root.parent().unwrap(),
            &[
                "clone",
                "-q",
                "-b",
                branch,
                self.origin().to_str().unwrap(),
                name,
            ],
        );
        dir
    }

    /// Commits on `branch` from a sibling clone and pushes it to the
    /// origin with raw git: the short sha pushed.
    fn diverge_branch(&self, branch: &str) -> String {
        let dir = self.sibling("other", branch);
        std::fs::write(dir.join("other.txt"), "other\n").unwrap();
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-qm", "other"]);
        git(&dir, &["push", "-q", "origin", branch]);
        git(&dir, &["rev-parse", "--short", "HEAD"])
    }

    /// [`Project::diverge_branch`] on the run's branch.
    fn diverge(&self) -> String {
        self.diverge_branch("feat/demo")
    }

    /// The origin's short head of `branch`.
    fn origin_head(&self, branch: &str) -> String {
        git(&self.origin(), &["rev-parse", "--short", branch])
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
        seed_log(
            &self.root,
            "2026-10-05T09:00",
            &card,
            &[&format!("commit={sha}")],
        );
        sha
    }
}

fn threads(path: &str) -> String {
    format!(
        r#"{{"data":{{"repository":{{"pullRequest":{{"state":"OPEN","isDraft":true,"reviewThreads":{{"nodes":[{{"id":"T1","isResolved":false,"path":"{path}","line":3,"originalLine":3,"comments":{{"nodes":[{{"databaseId":11,"author":{{"login":"al"}},"createdAt":"t","body":"why?"}}]}}}}]}}}}}}}}}}"#
    )
}

const NO_THREADS: &str = r#"{"data":{"repository":{"pullRequest":{"state":"OPEN","isDraft":true,"reviewThreads":{"nodes":[]}}}}}"#;

const THREADS_QUERY: &str = "query=query($owner: String!, $repo: String!, $pr: Int!, $after: String) {\n  repository(owner: $owner, name: $repo) {";

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

/// `gh issue list` looking for an open issue titled `title`.
fn find(title: &str) -> Vec<String> {
    [
        "issue",
        "list",
        "--state",
        "open",
        "--author",
        "@me",
        "--limit",
        "100",
        "--search",
        &format!("\"{title}\" in:title"),
        "--json",
        "number,title",
    ]
    .map(str::to_owned)
    .to_vec()
}

/// `gh issue view <n>` of an open issue without comments.
fn no_comments(gh: &FakeGh, n: &str) {
    gh.serve(
        &["issue", "view", n, "--json", GH_FIELDS],
        &gh_json(&format!(
            "#{n} An issue [OPEN]\nhttps://github.com/o/r/issues/{n}\nby someone\n\nbody\n"
        )),
    );
}

/// The area the demo run's brief names, `area:modbus`, is on issue `n`.
fn serve_links(gh: &FakeGh, n: &str) {
    serve_graph(gh, n, &["area:modbus"], &[]);
}

/// What the host answers for issue `n` to the node-id and the links
/// queries, which share one fixture key: its `labels` and the `blockers`
/// that block it.
fn serve_graph(gh: &FakeGh, n: &str, labels: &[&str], blockers: &[u64]) {
    let number = format!("number={n}");
    let labels: Vec<String> = labels
        .iter()
        .map(|l| format!(r#"{{"name":"{l}"}}"#))
        .collect();
    let blockers: Vec<String> = blockers
        .iter()
        .map(|b| {
            format!(
                r#"{{"number":{b},"url":"https://github.com/o/r/issues/{b}","state":"OPEN","repository":{{"nameWithOwner":"o/r"}}}}"#
            )
        })
        .collect();
    gh.serve(
        &[
            "api",
            "graphql",
            "-F",
            "owner={owner}",
            "-F",
            "repo={repo}",
            "-F",
            &number,
            "-f",
            "query=query($owner: String!, $repo: String!, $number: Int!) { repository(owner: $owner, name: $repo) { nameWithOwner issue(number: $number) {",
        ],
        &format!(
            r#"{{"data":{{"repository":{{"nameWithOwner":"o/r","issue":{{"id":"I{n}","state":"OPEN","milestone":null,"labels":{{"nodes":[{}]}},"parent":null,"blockedBy":{{"nodes":[{}]}},"issueDependenciesSummary":{{"blockedBy":{}}},"relatesTo":{{"nodes":[]}}}}}}}}}}"#,
            labels.join(","),
            blockers.join(","),
            blockers.len()
        ),
    );
}

/// The reads the host answers for an adopted issue `n` that differs from
/// the run's text and already carries its label.
fn adopted_issue(gh: &FakeGh, n: &str) {
    no_comments(gh, n);
    serve_links(gh, n);
}

/// `gh issue edit` giving adopted issue `n` the run's title and body.
fn edit_issue(n: &str, title: &str) -> Vec<String> {
    ["issue", "edit", n, "--title", title, "--body-file", "-"]
        .map(str::to_owned)
        .to_vec()
}

/// `gh pr list` looking for the open PR of `branch`.
fn pr_list(branch: &str) -> Vec<String> {
    [
        "pr",
        "list",
        "--head",
        branch,
        "--state",
        "open",
        "--json",
        "number,isCrossRepository",
    ]
    .map(str::to_owned)
    .to_vec()
}

fn serve(gh: &FakeGh, args: &[String], text: &str) {
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    gh.serve(&args, text);
}

/// Gates approved out of order are refused and write nothing.
fn out_of_order(p: &Project) {
    assert_eq!(
        p.ferrule(&["gate", "approve", "demo", "gate4"]).text(),
        "approve=refused reason=order missing=gate2 \
         hint=approve gate2 first: ferrule gate approve demo gate2\nexit=1"
    );
    assert_eq!(
        p.ferrule(&["gate", "approve", "demo", "gate2", "--issue", "new"])
            .text(),
        "approve=refused reason=order missing=arch \
         hint=approve arch first: ferrule gate approve demo arch\nexit=1"
    );
    assert_eq!(
        p.ferrule(&["gate", "approve", "demo", "epic"]).text(),
        "approve=refused reason=order missing=gate1 \
         hint=approve gate1 first: ferrule gate approve demo gate1 --epic\nexit=1"
    );
    assert!(!p.ok(&["board", "show", "demo"]).contains("gate"));
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

    out_of_order(p);

    p.ok(&["prompt", "render", "demo", "gate1"]);
    let gate1 = p.ok(&["gate", "approve", "demo", "gate1", "--lane", "full"]);
    assert!(
        gate1.starts_with("approve=done gate=gate1 spec-rev=5"),
        "{gate1}"
    );
    assert_eq!(p.field("demo", "gate1"), "2026-10-05");
    assert_eq!(p.field("demo", "lane"), "full");
    // Approved again a day later: the date stands.
    let again = p.at("2026-10-06T09:00", &["gate", "approve", "demo", "gate1"]);
    assert!(
        again.starts_with("approve=done gate=gate1 action=recorded"),
        "{again}"
    );
    assert_eq!(p.field("demo", "gate1"), "2026-10-05");
    p.ok(&["gate", "approve", "demo", "arch"]);
    assert_eq!(
        p.ferrule(&["gate", "approve", "demo", "gate1", "--epic"])
            .text(),
        "approve=refused reason=epic hint=the run is past gate 1 (arch or gate2 \
         recorded, or stage cards exist); an epic is chosen at gate 1 only; see \
         `ferrule run status demo`\nexit=1"
    );
    assert_eq!(p.field("demo", "lane"), "full");

    serve(gh, &find("Reconnect dropped Modbus clients"), "[]");
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
    serve_links(gh, "12");
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

    gate2_again(p);
}

/// Gate 2 approved again: what is missing is redone, nothing twice; the
/// recorded branch and the started run hold.
fn gate2_again(p: &Project) {
    let gh = &p.gh;
    // The worktree directory lost: approving again recreates it on the
    // existing branch and files nothing twice.
    std::fs::remove_dir_all(p.worktree()).unwrap();
    let again = p.ok(&["gate", "approve", "demo", "gate2"]);
    assert!(
        again.starts_with(
            "approve=done gate=gate2 issue=12 worktree=.ferrule/worktrees/demo stages=3"
        ),
        "{again}"
    );
    assert!(again.contains(" action=recorded"), "{again}");
    assert!(p.worktree().join("docs").is_dir());
    assert_eq!(p.field("demo", "issue"), "12");
    assert_eq!(gh.log().matches("issue create").count(), 1);

    // The recorded branch: the same --branch is fine, another one refused.
    let same = p.at(
        "2026-10-06T09:00",
        &["gate", "approve", "demo", "gate2", "--branch", "feat/demo"],
    );
    assert!(same.contains(" action=recorded"), "{same}");
    assert_eq!(p.field("demo", "gate2"), "2026-10-05");
    assert_eq!(
        p.ferrule(&["gate", "approve", "demo", "gate2", "--branch", "fix/demo"])
            .text(),
        "approve=refused reason=branch branch=feat/demo hint=the run's branch is \
         feat/demo; drop --branch or pass --branch feat/demo\nexit=1"
    );
    assert_eq!(p.field("demo", "branch"), "feat/demo");

    // Started: gate 1 and the epic are behind the run.
    assert_eq!(
        p.ferrule(&["gate", "approve", "demo", "gate1", "--epic"])
            .text(),
        "approve=refused reason=state state=inprogress hint=gate1 is approved only \
         while the parent card is open; see `ferrule run status demo`\nexit=1"
    );
    assert_eq!(p.field("demo", "kind"), "");
    assert_eq!(p.field("demo", "lane"), "full");
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
    serve(gh, &pr_list("feat/demo"), "[]");
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

/// A PR without checks is refused, not logged as a red `ci=1`.
fn no_checks_refused(p: &Project) {
    let gh = &p.gh;
    gh.serve_exit(
        &["pr", "checks", "34", "--watch"],
        "no checks reported on the 'feat/demo' branch\n",
        1,
    );
    assert_eq!(
        p.ferrule(&["ci", "wait", "demo"]).text(),
        "ci=refused reason=no-checks hint=the PR reports no checks yet; \
         rerun `ferrule ci wait demo` once CI has started\nexit=1"
    );
    assert!(!p.ok(&["board", "show", "demo"]).contains("ci=none"));
    // A call that read no checks (auth, a missing PR) is no red result.
    gh.serve_exit(&["pr", "checks", "34", "--watch"], "", 4);
    let failed = p.ferrule(&["ci", "wait", "demo"]).text();
    assert!(!failed.starts_with("ci=4"), "{failed}");
    assert!(!failed.ends_with("exit=0"), "{failed}");
    assert!(!p.ok(&["board", "show", "demo"]).contains("ci=4"));
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
    assert_eq!(
        p.ferrule(&["pr", "publish", "demo"]).text(),
        "publish=refused reason=gate4 \
         hint=approve gate4 first: ferrule gate approve <slug> gate4\nexit=1"
    );
    assert!(!gh.log().contains("pr ready"), "{}", gh.log());
    let gate4 = p.ok(&["gate", "approve", "demo", "gate4"]);
    assert!(gate4.starts_with("approve=done gate=gate4"), "{gate4}");
    assert_eq!(p.field("demo", "gate4"), "2026-10-05");
    // The tracker fails on the follow-up: the PR is ready, the follow-up
    // is not filed.
    let failed = p.ferrule(&["pr", "publish", "demo"]).text();
    assert!(failed.ends_with("exit=1"), "{failed}");
    assert_eq!(gh.log().matches("pr ready 34").count(), 1);
    // Rerun: the ready PR is left alone, pr=ready is logged once, the
    // follow-up is filed; a third run files nothing again.
    gh.serve(
        &["pr", "view", "34", "--json", "state,isDraft,body"],
        r#"{"state":"OPEN","isDraft":false,"body":"full"}"#,
    );
    serve(
        gh,
        &find("Share the backoff helper with the RTU client"),
        r#"[{"number":40,"title":"Share the backoff helper with the RTU client"}]"#,
    );
    serve(
        gh,
        &edit_issue("40", "Share the backoff helper with the RTU client"),
        "",
    );
    adopted_issue(gh, "40");
    for _ in 0..2 {
        let published = p.ok(&["pr", "publish", "demo"]);
        assert_eq!(published, "publish=done pr=34 follow-ups=#40");
    }
    let log = gh.log();
    assert_eq!(
        log.matches("pr edit 34 --body-file - --title Reconnect dropped Modbus clients")
            .count(),
        1,
        "{log}"
    );
    assert_eq!(log.matches("pr ready 34").count(), 1, "{log}");
    assert_eq!(
        log.matches("--search \"Share the backoff helper").count(),
        2,
        "{log}"
    );
    let shown = p.ok(&["board", "show", "demo"]);
    assert_eq!(shown.matches("pr=ready").count(), 1, "{shown}");
    assert_eq!(shown.matches("follow-ups=[40]").count(), 1, "{shown}");
    assert!(shown.contains("posted=follow-up issue=40 k=1"), "{shown}");

    gh.serve_exit(
        &["pr", "checks", "34", "--watch"],
        "test\tfail\t1m\thttps://x\ntest\tfail\t2m\thttps://x\n",
        1,
    );
    gh.serve(
        &[
            "api",
            "repos/{owner}/{repo}/commits/feat/demo",
            "--jq",
            ".sha",
        ],
        "feedbee\n",
    );
    let red = p.ok(&["ci", "wait", "demo"]);
    assert!(red.starts_with("ci=1 failed=test\n"), "{red}");
    no_checks_refused(p);
    gh.serve(
        &["pr", "checks", "34", "--watch"],
        "test\tpass\t1m\thttps://x\n",
    );
    std::fs::remove_file(gh.dir.path().join("fx").join("pr_checks_34_--watch.exit")).unwrap();
    let green = p.ok(&["ci", "wait", "demo"]);
    assert!(green.starts_with("ci=0\n"), "{green}");
    assert!(green.contains("ferrule run cleanup demo"), "{green}");
}

/// GA-R-001 — the stage gate approves while the parent is `inprogress` or
/// `inreview`.
#[test]
fn it_stage_gate_approves_during_implementation_and_review() {
    let p = Project::new();
    through_gate2(&p);
    first_stage(&p);
    p.ok(&["board", "move", "demo.s0", "inreview"]);
    p.ok(&["board", "move", "demo.s0", "done"]);
    std::fs::write(
        p.run_dir().join("review.verdict.md"),
        "# demo — review verdict (stage s1, pass 1)\n\nVerdict: clean\n",
    )
    .unwrap();
    std::fs::write(
        p.run_dir().join("plan.summary.md"),
        "# demo — plan summary (rev 1)\n\nDecide: 3 stages, s0 → s1 → s2.\n\n\
         | Stage | What | Pins | Files | Stop |\n|---|---|---|---|---|\n\
         | s0 | Land spec | all 4 entries | 2 (docs/specs/modbus) | — |\n\
         | s1 | Retry with backoff | MB-R-161 (1) | 1 (src/client.rs) | — |\n\
         | s2 | Render reconnecting | MB-R-042 (1) | 1 (src/tab.rs) | — |\n\n\
         Verification beyond unit tests:\n- none\n\n\
         Out of scope, raise separately:\n- none\n\nOpen: none\n",
    )
    .unwrap();
    seed_set(&p.root, "demo.s2", &["stop=yes"]);

    // s1, the parent in progress.
    p.ok(&["board", "move", "demo.s1", "inprogress"]);
    std::fs::create_dir_all(p.worktree().join("src")).unwrap();
    std::fs::write(p.worktree().join("src/client.rs"), "s1\n").unwrap();
    git(&p.worktree(), &["add", "."]);
    p.ok(&["board", "move", "demo.s1", "inreview"]);
    let stop = p.ferrule(&["stage", "stop", "demo", "s1"]).text();
    assert!(stop.starts_with("stop=5 reason=plan-marked"), "{stop}");
    p.ok(&[
        "prompt",
        "render",
        "demo",
        "stage",
        "--stage",
        "s1",
        "--reason",
        "plan-marked",
    ]);
    let approved = p.ferrule(&["gate", "approve", "demo", "stage"]).text();
    assert!(
        approved.starts_with("approve=done gate=stage"),
        "{approved}"
    );
    let done = p.in_wt(&["stage", "commit", "demo.s1", "-m", "Add backoff"]);
    assert!(done.starts_with("commit=done sha="), "{done}");
    p.ok(&["pr", "push", "demo"]);
    p.ok(&["board", "move", "demo.s1", "done"]);

    // s2, the parent in review.
    p.ok(&["board", "move", "demo", "inreview"]);
    p.ok(&["board", "move", "demo.s2", "inprogress"]);
    std::fs::write(p.worktree().join("src/tab.rs"), "s2\n").unwrap();
    git(&p.worktree(), &["add", "."]);
    p.ok(&["board", "move", "demo.s2", "inreview"]);
    let stop = p.ferrule(&["stage", "stop", "demo", "s2"]).text();
    assert!(stop.starts_with("stop=5 reason=plan-marked"), "{stop}");
    p.ok(&[
        "prompt",
        "render",
        "demo",
        "stage",
        "--stage",
        "s2",
        "--reason",
        "plan-marked",
    ]);
    let approved = p.ferrule(&["gate", "approve", "demo", "stage"]).text();
    assert!(
        approved.starts_with("approve=done gate=stage"),
        "{approved}"
    );
    assert_eq!(p.field("demo", "state"), "inreview");
    let done = p.in_wt(&["stage", "commit", "demo.s2", "-m", "Show status"]);
    assert!(done.starts_with("commit=done sha="), "{done}");
    let pushed = p.ferrule(&["pr", "push", "demo"]).text();
    assert!(pushed.starts_with("push=done sha="), "{pushed}");
    p.ok(&["board", "move", "demo.s2", "done"]);
    let g4 = p.ferrule(&["gate", "approve", "demo", "gate4"]).text();
    assert!(g4.starts_with("approve=done gate=gate4"), "{g4}");
    assert_eq!(p.field("demo", "gate4"), "2026-10-05");
}

/// GA-R-002 — the stage gate refuses any other parent state.
#[test]
fn it_stage_gate_refuses_other_parent_states() {
    for state in ["open", "done"] {
        let p = Project::new();
        seed_card(&p.root, "demo", state, &["gate2=2026-10-05"]);
        assert_eq!(
            p.ferrule(&["gate", "approve", "demo", "stage"]).text(),
            format!(
                "approve=refused reason=state state={state} hint=stage is approved only while the parent card is inprogress or inreview; see `ferrule run status demo`\nexit=1"
            )
        );
    }
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
    seed_set(&p.root, "demo", &["pr=34", "branch=feat/demo"]);
    assert_eq!(
        p.ferrule(&["pr", "publish", "demo"]).text(),
        "publish=refused reason=gate4 \
         hint=approve gate4 first: ferrule gate approve <slug> gate4\nexit=1"
    );
    seed_set(&p.root, "demo", &["gate4=2026-10-05"]);
    serve(&p.gh, &graphql("34"), &threads("src/x.rs"));
    assert_eq!(
        p.ferrule(&["pr", "publish", "demo"]).text(),
        "publish=refused reason=threads threads=1 \
         hint=resolve the open threads: ferrule guide show feedback\nexit=1"
    );
    // An unresolved thread on the second page still blocks the publish.
    serve(
        &p.gh,
        &graphql("34"),
        r#"{"data":{"repository":{"pullRequest":{"state":"OPEN","isDraft":true,"reviewThreads":{"pageInfo":{"hasNextPage":true,"endCursor":"C1"},"nodes":[]}}}}}"#,
    );
    let mut second = graphql("34");
    second.splice(8..8, ["-f".to_owned(), "after=C1".to_owned()]);
    serve(&p.gh, &second, &threads("src/x.rs"));
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
    seed_set(&p.root, "demo", &["branch=main"]);
    assert_eq!(
        p.ferrule(&["pr", "push", "demo"]).text(),
        "push=refused reason=main hint=push from a branch other than main\nexit=1"
    );
    seed_set(&p.root, "demo", &["branch=feat/demo"]);
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
    p.ok(&["board", "set", "demo", "lane=fast", "arch=skipped"]);
    serve(
        &p.gh,
        &find("Reconnect dropped Modbus clients"),
        r#"[{"number":12,"title":"Reconnect dropped Modbus clients"}]"#,
    );
    serve(
        &p.gh,
        &edit_issue("12", "Reconnect dropped Modbus clients"),
        "",
    );
    adopted_issue(&p.gh, "12");
    let gate2 = p.ok(&["gate", "approve", "demo", "gate2"]);
    assert!(gate2.contains("issue=12 "), "adopted: {gate2}");
    assert_eq!(p.field("demo", "gate1"), "2026-10-05", "pending: dated");
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

    p.ok(&["gate", "approve", "demo", "gate4"]);
    assert_eq!(
        p.ok(&["stage", "replan", "demo", "start"]),
        "replan=started from=s1"
    );
    assert_eq!(
        p.field("demo", "gate4"),
        "",
        "approved anew after a re-plan"
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
        "card-show=refused reason=card hint=check the slug with `ferrule board list`\nexit=1"
    );
    let parent = p.ok(&["board", "show", "demo"]);
    assert!(
        parent.contains("spec-delta MB-R-170 added origin=impl retired"),
        "{parent}"
    );
    assert!(parent.contains("replan=approved"), "{parent}");
}

#[test]
fn it_replan_keeps_a_stage_committed_past_an_uncommitted_one() {
    let p = Project::new();
    p.ok(&["run", "init", "demo"]);
    p.artifacts();
    p.ok(&["board", "set", "demo", "lane=fast", "arch=skipped"]);
    serve(
        &p.gh,
        &find("Reconnect dropped Modbus clients"),
        r#"[{"number":12,"title":"Reconnect dropped Modbus clients"}]"#,
    );
    serve(
        &p.gh,
        &edit_issue("12", "Reconnect dropped Modbus clients"),
        "",
    );
    adopted_issue(&p.gh, "12");
    p.ok(&["gate", "approve", "demo", "gate2"]);
    seed_log(&p.root, "2026-10-05T09:00", "demo.s2", &["commit=abc1234"]);
    assert_eq!(
        p.ok(&["stage", "replan", "demo", "start"]),
        "replan=started from=s0"
    );
    // Redefining the committed s2 is refused and changes nothing.
    std::fs::write(p.run_dir().join("plan.md"), PLAN).unwrap();
    assert_eq!(
        p.ferrule(&["stage", "replan", "demo", "approve"]).text(),
        "replan=refused reason=committed stage=s2 \
         hint=s2 is committed and stays; number the new plan's stages around it\nexit=1"
    );
    std::fs::write(
        p.run_dir().join("plan.md"),
        PLAN.replace("| s2 | Show status | src/tab.rs, src/view.rs | s1 |\n", ""),
    )
    .unwrap();
    assert_eq!(
        p.ok(&["stage", "replan", "demo", "approve"]),
        "replan=approved from=s0 stages=2"
    );
    assert!(
        p.ok(&["board", "show", "demo.s2"])
            .contains("commit=abc1234")
    );
}

#[test]
fn it_replan_approve_is_refused_twice() {
    let p = Project::new();
    p.ok(&["run", "init", "demo"]);
    p.artifacts();
    p.ok(&["board", "set", "demo", "lane=fast", "arch=skipped"]);
    serve(
        &p.gh,
        &find("Reconnect dropped Modbus clients"),
        r#"[{"number":12,"title":"Reconnect dropped Modbus clients"}]"#,
    );
    serve(
        &p.gh,
        &edit_issue("12", "Reconnect dropped Modbus clients"),
        "",
    );
    adopted_issue(&p.gh, "12");
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

/// RU-R-006, RU-R-007 — `run init --issue` records `named-issue` and not
/// `issue`; both fields name the command that owns them.
#[test]
fn it_init_records_the_named_issue() {
    let p = Project::new();
    p.gh.serve(
        &["issue", "view", "7", "--json", GH_FIELDS],
        &gh_json(&std::fs::read_to_string(fixtures().join("issue/plain.txt")).unwrap()),
    );
    assert_eq!(
        p.ok(&["run", "init", "demo", "--issue", "7"]),
        "init=done slug=demo"
    );
    assert_eq!(p.field("demo", "named-issue"), "7");
    assert_eq!(p.field("demo", "issue"), "");
    let named = p.ferrule(&["board", "set", "demo", "named-issue=8"]).text();
    assert!(
        named.contains("ferrule run init <slug> --issue <n>"),
        "{named}"
    );
    let issue = p.ferrule(&["board", "set", "demo", "issue=8"]).text();
    assert!(
        issue.contains("ferrule gate approve <slug> gate2 or ferrule issue post <slug>"),
        "{issue}"
    );
}

/// RU-R-007 — gate 2 records the filed issue on the card.
#[test]
fn it_gate2_records_the_filed_issue() {
    let p = Project::new();
    through_gate2(&p);
    assert_eq!(p.field("demo", "issue"), "12");
}

/// RU-E-003 — `run init --request` of a slice issue is a request run: no
/// epic lookup, no slice files.
#[test]
fn it_init_request_of_a_slice_is_no_slice() {
    let p = Project::new();
    p.gh.serve(
        &["issue", "view", "71", "--json", GH_FIELDS],
        &gh_json(&std::fs::read_to_string(fixtures().join("issue/slice.txt")).unwrap()),
    );
    assert_eq!(
        p.ok(&["run", "init", "rq", "--request", "--issue", "71"]),
        "init=done slug=rq"
    );
    assert_eq!(p.field("rq", "kind"), "request");
    assert_eq!(p.field("rq", "epic"), "");
    let dir = p.root.join(".ferrule/runs/rq");
    assert!(!dir.join("slice-line.md").exists());
    assert!(!dir.join("epic.md").exists());
    assert!(!p.gh.log().contains("issue view 70"), "{}", p.gh.log());
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
    seed_set(&p.root, "demo", &["issue=12"]);
    assert_eq!(p.ok(&["issue", "post", "demo"]), "issue=12 action=recorded");
    serve(gh, &find("RTU clients reconnect"), "[]");
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
    no_comments(gh, "12");
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
    adopted_issue(gh, "41");
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
    seed_set(&p.root, "demo", &["issue=12", "pr=34"]);
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
    seed_set(&p.root, "demo", &["branch=feat/demo"]);
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

/// A run `demo` with stage `demo.s1` in review and its worktree on `feat/demo`.
fn stage_project() -> Project {
    let p = Project::new();
    p.ok(&["run", "init", "demo"]);
    seed_set(&p.root, "demo", &["branch=feat/demo"]);
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
    p.ok(&["board", "move", "demo.s1", "inprogress"]);
    p.ok(&["board", "move", "demo.s1", "inreview"]);
    p
}

/// `ferrule` run in `cwd` with a git identity.
fn in_dir(p: &Project, cwd: &Path, args: &[&str]) -> String {
    p.gh.on(Run::new(args))
        .env("CLAUDE_PROJECT_DIR", &p.root)
        .env("FERRULE_NOW", "2026-10-05T09:00")
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t")
        .cwd(cwd)
        .text()
}

/// GW-R-001, GW-R-002, S1 — nothing staged on a dirty tree is refused as such, with the `git add` hint.
#[test]
fn it_commit_refuses_with_nothing_staged_on_a_dirty_tree() {
    let p = stage_project();
    let wt = p.worktree();
    std::fs::create_dir(wt.join("sub")).unwrap();
    std::fs::write(wt.join("sub/keep.txt"), "k\n").unwrap();
    std::fs::write(wt.join("a.txt"), "a\n").unwrap();
    git(&wt, &["add", "."]);
    git(&wt, &["commit", "-q", "-m", "base"]);
    let head = git(&wt, &["rev-parse", "--short", "HEAD"]);
    let check = |got: String| {
        let line = format!("commit=refused reason=nothing-staged head={head} hint=");
        assert!(got.starts_with(&line), "{got}");
        assert!(got.contains("git add"), "{got}");
        assert!(got.ends_with("exit=1"), "{got}");
    };

    std::fs::write(wt.join("a.txt"), "changed\n").unwrap();
    let got = in_dir(&p, &wt, &["stage", "commit", "demo.s1", "-m", "x"]);
    check(got);

    git(&wt, &["checkout", "--", "a.txt"]);
    std::fs::write(wt.join("new.txt"), "n\n").unwrap();
    let got = in_dir(&p, &wt, &["stage", "commit", "demo.s1", "-m", "x"]);
    check(got);

    std::fs::remove_file(wt.join("new.txt")).unwrap();
    std::fs::write(wt.join("a.txt"), "changed\n").unwrap();
    let got = in_dir(
        &p,
        &wt.join("sub"),
        &["stage", "commit", "demo.s1", "-m", "x"],
    );
    check(got);
}

/// GW-R-001, GW-R-003, S2 — nothing staged on a clean tree names the head and says the changes may be committed.
#[test]
fn it_commit_refuses_with_nothing_staged_on_a_clean_tree() {
    let p = stage_project();
    let wt = p.worktree();
    std::fs::write(wt.join("a.txt"), "a\n").unwrap();
    git(&wt, &["add", "a.txt"]);
    git(&wt, &["commit", "-q", "-m", "outside"]);
    let head = git(&wt, &["rev-parse", "--short", "HEAD"]);
    let got = in_dir(&p, &wt, &["stage", "commit", "demo.s1", "-m", "x"]);
    let line = format!("commit=refused reason=nothing-staged head={head} hint=");
    assert!(got.starts_with(&line), "{got}");
    assert!(got.contains("may already be committed"), "{got}");
    assert!(got.ends_with("exit=1"), "{got}");
}

/// GW-R-004, S3 — `--no-run` applies the nothing-staged check.
#[test]
fn it_commit_no_run_refuses_with_nothing_staged() {
    let p = stage_project();
    let wt = p.worktree();
    let head = git(&wt, &["rev-parse", "--short", "HEAD"]);
    let got = in_dir(&p, &wt, &["stage", "commit", "--no-run", "-m", "x"]);
    assert!(
        got.starts_with(&format!(
            "commit=refused reason=nothing-staged head={head} hint="
        )),
        "{got}"
    );
    assert!(got.ends_with("exit=1"), "{got}");
}

/// A hook that refuses a staged change is still `reason=git`, and its hint no longer advises `git add`.
#[cfg(unix)]
#[test]
fn it_commit_reports_reason_git_for_a_refusing_hook() {
    use std::os::unix::fs::PermissionsExt;
    let p = stage_project();
    let wt = p.worktree();
    let hook = p.root.join(".git/hooks/pre-commit");
    std::fs::write(&hook, "#!/bin/sh\nexit 1\n").unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(wt.join("a.txt"), "a\n").unwrap();
    git(&wt, &["add", "a.txt"]);
    let got = in_dir(&p, &wt, &["stage", "commit", "demo.s1", "-m", "x"]);
    assert!(got.starts_with("commit=refused reason=git hint="), "{got}");
    assert!(!got.contains("git add"), "{got}");
    assert!(got.contains("stderr"), "{got}");
}

/// GW-E-004 — a merge in progress whose index equals HEAD is concluded by the commit, not refused.
#[test]
fn it_commit_concludes_a_merge_whose_index_equals_head() {
    let p = stage_project();
    let wt = p.worktree();
    std::fs::write(wt.join("a.txt"), "a\n").unwrap();
    git(&wt, &["add", "a.txt"]);
    git(&wt, &["commit", "-q", "-m", "base"]);
    git(&wt, &["checkout", "-q", "-b", "side"]);
    std::fs::write(wt.join("s.txt"), "s\n").unwrap();
    git(&wt, &["add", "s.txt"]);
    git(&wt, &["commit", "-q", "-m", "side"]);
    git(&wt, &["checkout", "-q", "feat/demo"]);
    std::fs::write(wt.join("b.txt"), "b\n").unwrap();
    git(&wt, &["add", "b.txt"]);
    git(&wt, &["commit", "-q", "-m", "own"]);
    git(
        &wt,
        &["merge", "--no-commit", "--no-ff", "-s", "ours", "side"],
    );
    let quiet = std::process::Command::new("git")
        .args(["diff", "--cached", "--quiet"])
        .current_dir(&wt)
        .status()
        .unwrap();
    assert_eq!(quiet.code(), Some(0));
    assert!(!git(&wt, &["rev-parse", "-q", "--verify", "MERGE_HEAD"]).is_empty());

    let got = in_dir(&p, &wt, &["stage", "commit", "demo.s1", "-m", "merge"]);
    assert!(got.starts_with("commit=done sha="), "{got}");
    let parents = git(&wt, &["rev-list", "--parents", "-n", "1", "HEAD"]);
    assert_eq!(parents.split_whitespace().count(), 3, "{parents}");
}

/// A commit made outside ferrule cannot be logged as a stage's `commit=` with `board log`.
#[test]
fn it_board_log_refuses_an_outside_commit() {
    let p = stage_project();
    let wt = p.worktree();
    std::fs::write(wt.join("a.txt"), "a\n").unwrap();
    git(&wt, &["add", "a.txt"]);
    git(&wt, &["commit", "-q", "-m", "outside"]);
    let sha = git(&wt, &["rev-parse", "--short", "HEAD"]);
    let before = p.field("demo.s1", "log:commit");
    let got = p
        .ferrule(&["board", "log", "demo.s1", &format!("commit={sha}")])
        .text();
    assert!(got.contains("reason=owner"), "{got}");
    assert_eq!(p.field("demo.s1", "log:commit"), before);
}

/// `stage_project` with a `files` list on the stage card.
fn adopt_project() -> Project {
    let p = stage_project();
    seed_set(&p.root, "demo.s1", &["files=[a.txt, b.txt]"]);
    p
}

/// Commits `files` in the worktree and returns the short sha.
fn outside_commit(p: &Project, files: &[&str], message: &str) -> String {
    let wt = p.worktree();
    for f in files {
        std::fs::write(wt.join(f), format!("{f}\n")).unwrap();
    }
    let mut add = vec!["add"];
    add.extend(files);
    git(&wt, &add);
    git(&wt, &["commit", "-q", "-m", message]);
    git(&wt, &["rev-parse", "--short", "HEAD"])
}

/// Logs `sha` as a `commit=` of `demo.s1`.
fn log_commit_of(p: &Project, sha: &str) {
    seed_log(
        &p.root,
        "2026-10-05T09:00",
        "demo.s1",
        &[&format!("commit={sha}")],
    );
}

fn adopt_in(p: &Project, sha: &str) -> String {
    in_dir(
        p,
        &p.worktree(),
        &["stage", "commit", "demo.s1", "--adopt", sha],
    )
}

fn assert_adopt_refused(p: &Project, got: &str, line: &str, before: &str) {
    assert!(
        got.starts_with(&format!("commit=refused {line}hint=")),
        "{got}"
    );
    assert!(got.ends_with("exit=1"), "{got}");
    assert_eq!(p.field("demo.s1", "log:commit"), before);
}

/// Adopts `sha`, which must be refused with `line` and log nothing.
fn adopt_refused(p: &Project, sha: &str, line: &str) -> String {
    let before = p.field("demo.s1", "log:commit");
    let got = adopt_in(p, sha);
    assert_adopt_refused(p, &got, line, &before);
    got
}

/// GW-R-030 — adopting HEAD logs it as the card's `commit=` and creates no commit.
#[test]
fn it_adopt_logs_head_without_a_new_commit() {
    let p = adopt_project();
    let wt = p.worktree();
    let sha = outside_commit(&p, &["a.txt"], "outside");
    let full = git(&wt, &["rev-parse", "HEAD"]);
    let log = git(&wt, &["log", "--format=%H"]);
    let got = adopt_in(&p, &sha);
    assert_eq!(got, format!("commit=done sha={sha} card=demo.s1\nexit=0"));
    assert_eq!(git(&wt, &["rev-parse", "HEAD"]), full);
    assert_eq!(git(&wt, &["log", "--format=%H"]), log);
    assert_eq!(git(&wt, &["log", "-1", "--format=%B"]), "outside");
    assert_eq!(p.field("demo.s1", "log:commit"), sha);
}

/// GW-R-031, S8 — adopting needs a card in review, a clean worktree and the run's branch.
#[test]
fn it_adopt_refuses_a_wrong_state_dirty_tree_or_branch() {
    let p = adopt_project();
    let sha = outside_commit(&p, &["a.txt"], "outside");
    p.ok(&["board", "new", "demo.s2", "title=Add b"]);
    let got = in_dir(
        &p,
        &p.worktree(),
        &["stage", "commit", "demo.s2", "--adopt", &sha],
    );
    assert!(
        got.starts_with("commit=refused reason=state state=open hint="),
        "{got}"
    );

    std::fs::write(p.worktree().join("a.txt"), "changed\n").unwrap();
    adopt_refused(&p, &sha, "reason=dirty ");
    git(&p.worktree(), &["checkout", "-q", "--", "a.txt"]);

    std::fs::write(p.worktree().join("new.txt"), "n\n").unwrap();
    adopt_refused(&p, &sha, "reason=dirty ");
    std::fs::remove_file(p.worktree().join("new.txt")).unwrap();

    git(&p.worktree(), &["checkout", "-q", "--detach"]);
    adopt_refused(&p, &sha, "reason=branch want=feat/demo ");
    git(&p.worktree(), &["checkout", "-q", "feat/demo"]);

    seed_set(&p.root, "demo", &["branch="]);
    adopt_refused(&p, &sha, "reason=branch want= ");
}

/// GW-R-032, S6 — adopting needs the branch's HEAD.
#[test]
fn it_adopt_refuses_a_sha_that_is_not_head() {
    let p = adopt_project();
    let first = outside_commit(&p, &["a.txt"], "first");
    log_commit_of(&p, &first);
    let head = outside_commit(&p, &["b.txt"], "second");
    let line = format!("reason=not-head head={head} ");
    adopt_refused(&p, &format!("{head}^"), &line);
    adopt_refused(&p, "nosuch", &line);
}

/// GW-R-032, S5, GW-E-003 — a merge commit is not adopted.
#[test]
fn it_adopt_refuses_a_merge_commit() {
    let p = adopt_project();
    let wt = p.worktree();
    git(&wt, &["checkout", "-q", "-b", "side"]);
    outside_commit(&p, &["b.txt"], "side");
    git(&wt, &["checkout", "-q", "feat/demo"]);
    git(&wt, &["merge", "-q", "--no-ff", "-m", "merge", "side"]);
    let head = git(&wt, &["rev-parse", "--short", "HEAD"]);
    adopt_refused(&p, &head, "reason=merge parents=2 ");
}

/// GW-R-033 — a branch point that cannot be found refuses with its own reason instead of naming a commit.
#[test]
fn it_adopt_refuses_when_the_branch_point_is_unknown() {
    let p = adopt_project();
    let wt = p.worktree();
    git(&wt, &["checkout", "-q", "--orphan", "other"]);
    git(&wt, &["rm", "-rfq", "--ignore-unmatch", "."]);
    seed_set(&p.root, "demo", &["branch=other"]);
    outside_commit(&p, &["a.txt"], "first");
    let head = outside_commit(&p, &["b.txt"], "second");
    let got = adopt_refused(&p, &head, "reason=no-merge-base ");
    assert!(!got.contains("unlogged"), "{got}");
}

/// GW-E-011 — a HEAD already logged is not adopted again.
#[test]
fn it_adopt_refuses_a_head_already_logged() {
    let p = adopt_project();
    let sha = outside_commit(&p, &["a.txt"], "outside");
    assert!(adopt_in(&p, &sha).starts_with("commit=done"));
    let got = adopt_in(&p, &sha);
    assert!(
        got.starts_with("commit=refused reason=adopted card=demo.s1 hint="),
        "{got}"
    );
    assert!(got.ends_with("exit=1"), "{got}");
}

/// GW-R-033, S7 — every commit before HEAD must be logged on a card of the run.
#[test]
fn it_adopt_refuses_an_unlogged_earlier_commit() {
    let p = adopt_project();
    let first = outside_commit(&p, &["a.txt"], "first");
    let head = outside_commit(&p, &["b.txt"], "second");
    let line = format!("reason=unlogged sha={first} ");
    adopt_refused(&p, &head, &line);
}

/// GW-R-034, S3 — a file outside the `files` list, either side of a rename, is refused.
#[test]
fn it_adopt_refuses_a_file_outside_the_files_list() {
    let p = adopt_project();
    let head = outside_commit(&p, &["a.txt", "c.txt"], "both");
    adopt_refused(&p, &head, "reason=files path=c.txt ");

    let p = adopt_project();
    let first = outside_commit(&p, &["a.txt"], "a");
    log_commit_of(&p, &first);
    git(&p.worktree(), &["mv", "a.txt", "c.txt"]);
    git(&p.worktree(), &["commit", "-q", "-m", "rename"]);
    let head = git(&p.worktree(), &["rev-parse", "--short", "HEAD"]);
    adopt_refused(&p, &head, "reason=files path=c.txt ");

    let p = adopt_project();
    let first = outside_commit(&p, &["c.txt"], "c");
    log_commit_of(&p, &first);
    git(&p.worktree(), &["mv", "c.txt", "a.txt"]);
    git(&p.worktree(), &["commit", "-q", "-m", "rename"]);
    let head = git(&p.worktree(), &["rev-parse", "--short", "HEAD"]);
    adopt_refused(&p, &head, "reason=files path=c.txt ");
}

/// GW-R-035, S4 — attribution trailers in the message are refused with an amend hint.
#[test]
fn it_adopt_refuses_attribution_trailers() {
    let p = adopt_project();
    let trailer = format!("{}: A <a@b>", ["Co", "Authored", "By"].join("-"));
    let head = outside_commit(&p, &["a.txt"], &format!("outside\n\n{trailer}"));
    let got = adopt_refused(&p, &head, "reason=trailers ");
    assert!(got.contains("amend"), "{got}");
}

/// GW-E-012 — a commit that touches no file is adopted.
#[test]
fn it_adopt_accepts_a_commit_touching_no_file() {
    let p = adopt_project();
    git(
        &p.worktree(),
        &["commit", "-q", "--allow-empty", "-m", "empty"],
    );
    let head = git(&p.worktree(), &["rev-parse", "--short", "HEAD"]);
    let got = adopt_in(&p, &head);
    assert!(got.starts_with("commit=done"), "{got}");
}

/// GW-R-037 — `--adopt` conflicts with `-m` and `--no-run`; `-m` is required without it.
#[test]
fn it_adopt_conflicts_with_message_and_no_run() {
    let p = adopt_project();
    let wt = p.worktree();
    for (args, want) in [
        (
            &["stage", "commit", "demo.s1", "--adopt", "x", "-m", "y"][..],
            "the argument '--adopt <SHA>' cannot be used with '--message <MESSAGE>'",
        ),
        (
            &["stage", "commit", "--adopt", "x", "--no-run"][..],
            "the argument '--adopt <SHA>' cannot be used with '--no-run'",
        ),
        (
            &["stage", "commit", "demo.s1"][..],
            "the following required arguments were not provided:\n  --message <MESSAGE>",
        ),
    ] {
        let out =
            p.gh.on(Run::new(args))
                .env("CLAUDE_PROJECT_DIR", &p.root)
                .cwd(&wt)
                .output();
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(err.contains(want), "{err}");
        assert_eq!(out.status.code(), Some(2), "{err}");
    }
}

/// A run whose first stage is pushed and done, with the draft PR open: the
/// project and the first push's output.
fn pushed_run() -> (Project, String) {
    let p = Project::new();
    p.ok(&["run", "init", "demo"]);
    p.artifacts();
    p.ok(&["board", "set", "demo", "lane=fast", "arch=skipped"]);
    serve(
        &p.gh,
        &find("Reconnect dropped Modbus clients"),
        r#"[{"number":12,"title":"Reconnect dropped Modbus clients"}]"#,
    );
    serve(
        &p.gh,
        &edit_issue("12", "Reconnect dropped Modbus clients"),
        "",
    );
    adopted_issue(&p.gh, "12");
    p.ok(&["gate", "approve", "demo", "gate2"]);
    p.land("s0", "docs/specs/modbus/requirements.md");
    p.gh.serve(
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
    serve(&p.gh, &pr_list("feat/demo"), "[]");
    let first = p.ok(&["pr", "push", "demo"]);
    p.gh.serve(
        &["pr", "view", "34", "--json", "state,isDraft,body"],
        r#"{"state":"OPEN","isDraft":true,"body":"stale"}"#,
    );
    p.gh.serve(&["pr", "edit", "34", "--body-file", "-"], "");
    p.ok(&["board", "move", "demo.s0", "done"]);
    (p, first)
}

/// A diverged run at a new stage commit: the project, the local head and
/// the remote head, both short.
fn diverged_run() -> (Project, String, String) {
    let (p, _) = pushed_run();
    let remote = p.diverge();
    let local = p.land("s1", "src/client.rs");
    (p, local, remote)
}

fn push_text(p: &Project) -> String {
    p.ferrule(&["pr", "push", "demo"]).text()
}

/// Raw git in `dir` without the success assert: a conflicting rebase exits
/// non-zero.
fn git_status(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new("git")
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
        .unwrap()
}

/// A sibling clone commits `file` on `main` and pushes: the short sha.
fn push_main(p: &Project, file: &str, text: &str) -> String {
    let dir = p.sibling("mover", "main");
    let path = dir.join(file);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, text).unwrap();
    git(&dir, &["add", "."]);
    git(&dir, &["commit", "-qm", "moved"]);
    git(&dir, &["push", "-q", "origin", "main"]);
    git(&dir, &["rev-parse", "--short", "HEAD"])
}

/// `ferrule run rebase demo <args>` in the run's worktree.
fn rebase(p: &Project, args: &[&str]) -> String {
    let mut all = vec!["run", "rebase", "demo"];
    all.extend_from_slice(args);
    p.gh.on(Run::new(all))
        .env("CLAUDE_PROJECT_DIR", &p.root)
        .env("FERRULE_NOW", "2026-10-05T09:00")
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t")
        .env("GIT_CONFIG_COUNT", "1")
        .env("GIT_CONFIG_KEY_0", "commit.gpgsign")
        .env("GIT_CONFIG_VALUE_0", "false")
        .cwd(p.worktree())
        .text()
}

/// A pushed run whose first two stages are done.
fn pushed_s1() -> Project {
    let (p, _) = pushed_run();
    p.land("s1", "src/client.rs");
    let got = push_text(&p);
    assert!(got.starts_with("push=done "), "{got}");
    p.ok(&["board", "move", "demo.s1", "done"]);
    p
}

/// Moves `main` so that it conflicts with stage s1 and starts the rebase.
fn conflicted(p: &Project) {
    push_main(p, "src/client.rs", "base\n");
    let got = rebase(p, &[]);
    assert!(got.starts_with("rebase=conflict paths=1 hint="), "{got}");
    assert!(
        got.contains("GIT_EDITOR=true git rebase --continue"),
        "{got}"
    );
    assert!(got.contains("ferrule run rebase"), "{got}");
    assert!(got.ends_with("exit=1"), "{got}");
}

fn wt_head(p: &Project) -> String {
    git(&p.worktree(), &["rev-parse", "--short", "HEAD"])
}

/// The log of the run card and both stage cards.
fn cards(p: &Project) -> String {
    ["demo", "demo.s0", "demo.s1"]
        .iter()
        .map(|c| p.ok(&["board", "show", c]))
        .collect::<Vec<_>>()
        .join("\n")
}

/// GW-R-007, GW-R-013 — a diverged branch is refused with both heads and a hint that never forces.
#[test]
fn it_push_refuses_a_diverged_branch() {
    let (p, local, remote) = diverged_run();
    let got = push_text(&p);
    let line = format!("push=refused reason=diverged local={local} remote={remote} hint=");
    assert!(got.starts_with(&line), "{got}");
    assert!(got.contains("outside ferrule"), "{got}");
    assert!(got.contains("rebas"), "{got}");
    assert!(got.contains("ferrule run rebase"), "{got}");
    assert!(got.contains("never force-push by hand"), "{got}");
    assert!(got.ends_with("exit=1"), "{got}");
    assert_eq!(p.origin_head("feat/demo"), remote);
}

/// GW-R-007 — a branch only behind the remote is diverged too.
#[test]
fn it_push_refuses_a_branch_only_behind_the_remote() {
    let (p, _) = pushed_run();
    let remote = p.diverge();
    let local = git(&p.worktree(), &["rev-parse", "--short", "HEAD"]);
    let got = push_text(&p);
    let line = format!("push=refused reason=diverged local={local} remote={remote} hint=");
    assert!(got.starts_with(&line), "{got}");
}

/// GW-R-036, S1 — an adopted commit is reviewed, gauntleted and pushed as a fast-forward.
#[test]
fn it_adopted_commit_pushes_as_a_fast_forward() {
    let (p, _) = pushed_run();
    let wt = p.worktree();
    p.ok(&["board", "move", "demo.s1", "inprogress"]);
    std::fs::create_dir_all(wt.join("src")).unwrap();
    std::fs::write(wt.join("src/client.rs"), "client\n").unwrap();
    git(&wt, &["add", "."]);
    git(&wt, &["commit", "-q", "-m", "outside"]);
    let sha = git(&wt, &["rev-parse", "--short", "HEAD"]);
    p.ok(&["board", "move", "demo.s1", "inreview"]);

    let refused = p.in_wt(&["stage", "commit", "demo.s1", "-m", "x"]);
    let line = format!("commit=refused reason=nothing-staged head={sha} hint=");
    assert!(refused.starts_with(&line), "{refused}");
    let adopted = p.in_wt(&["stage", "commit", "demo.s1", "--adopt", &sha]);
    assert_eq!(
        adopted,
        format!("commit=done sha={sha} card=demo.s1\nexit=0")
    );
    let diff = p.in_wt(&["run", "diff", "demo", "--path", "src/client.rs"]);
    assert!(diff.contains("src/client.rs"), "{diff}");

    let log = p.run_dir().join("gauntlet.log");
    let _ = std::fs::remove_file(&log);
    let pushed = push_text(&p);
    assert!(pushed.starts_with("push=done sha="), "{pushed}");
    assert!(log.is_file());
    assert_eq!(p.origin_head("feat/demo"), sha);
    assert_eq!(git(&wt, &["log", "-1", "--format=%B"]), "outside");
    assert!(!p.ok(&["board", "show", "demo"]).contains("replan"));
}

/// GW-R-009 — the divergence check runs before the gauntlet.
#[test]
fn it_diverged_push_runs_no_gauntlet() {
    let (p, _, _) = diverged_run();
    let log = p.run_dir().join("gauntlet.log");
    let _ = std::fs::remove_file(&log);
    let got = push_text(&p);
    assert!(got.starts_with("push=refused reason=diverged "), "{got}");
    assert!(!log.exists());
}

/// A retry of a diverged push gets the same refusal.
#[test]
fn it_diverged_push_refuses_again_on_retry() {
    let (p, _, _) = diverged_run();
    let first = push_text(&p);
    assert!(
        first.starts_with("push=refused reason=diverged "),
        "{first}"
    );
    assert_eq!(push_text(&p), first);
}

/// S4 — the diverged refusal's heads feed the blocked stop, which offers no re-plan.
#[test]
fn it_diverged_refusal_feeds_the_blocked_stop() {
    let (p, local, remote) = diverged_run();
    let got = push_text(&p);
    let head = |key: &str| {
        let at = got.find(&format!("{key}=")).unwrap() + key.len() + 1;
        got[at..].split_whitespace().next().unwrap().to_owned()
    };
    let (l, r) = (head("local"), head("remote"));
    assert_eq!((l.as_str(), r.as_str()), (local.as_str(), remote.as_str()));
    let stop = p
        .ferrule(&[
            "prompt", "render", "demo", "blocked", "--local", &l, "--remote", &r,
        ])
        .text();
    assert!(
        stop.contains(&format!("local={local} remote={remote}")),
        "{stop}"
    );
    assert!(!stop.contains("re-plan"), "{stop}");
    assert!(stop.contains("never force-push"), "{stop}");
    assert!(stop.contains("a) rebase through ferrule"), "{stop}");
    assert!(
        stop.contains("b) resolved outside ferrule, push again"),
        "{stop}"
    );
    assert!(stop.contains("c) stop"), "{stop}");
}

/// Serves the issue and the PR of `pushed_run` as the host would show them.
fn serve_host(p: &Project) {
    let open = "#12 Reconnect dropped Modbus clients [OPEN]\nhttps://github.com/o/r/issues/12\nby someone\n\nbody\n\n--- 0 comment(s) ---\n";
    p.gh.serve(
        &["issue", "view", "12", "--json", GH_FIELDS],
        &gh_json(open),
    );
    p.gh.serve(
        &["pr", "view", "34", "--json", "state,isDraft,body"],
        r#"{"state":"OPEN","isDraft":true,"body":"x"}"#,
    );
}

/// GW-R-018 — reconcile reports a diverged branch before the host rows.
#[test]
fn it_reconcile_reports_a_diverged_branch() {
    let (p, local, remote) = diverged_run();
    serve_host(&p);
    let out = p.ferrule(&["run", "reconcile", "demo"]).text();
    let row = format!("differ row=diverged feat/demo local={local} remote={remote}\n");
    assert!(out.contains(&row), "{out}");
    let pr = out.find("differ row=pr #").expect(&out);
    assert!(out.find(&row).unwrap() < pr, "{out}");
    assert!(out.ends_with("exit=1"), "{out}");
}

/// GW-R-019 — a remote that cannot be read adds no diverged row.
#[test]
fn it_reconcile_adds_no_row_for_an_unreadable_remote() {
    let (p, _, _) = diverged_run();
    serve_host(&p);
    git(
        &p.root,
        &["remote", "set-url", "origin", "/nonexistent/origin.git"],
    );
    let out = p.ferrule(&["run", "reconcile", "demo"]).text();
    assert!(!out.contains("diverged"), "{out}");
}

/// GW-R-023 — `--local` skips the divergence check.
#[test]
fn it_reconcile_local_skips_the_divergence_check() {
    let (p, _, _) = diverged_run();
    let out = p.ferrule(&["run", "reconcile", "demo", "--local"]).text();
    assert!(!out.contains("diverged"), "{out}");
    assert!(out.contains("reconcile=clean"), "{out}");
}

/// GW-R-021 — the dirty and sha refusals come before the diverged one.
#[test]
fn it_dirty_and_sha_refusals_precede_diverged() {
    let (p, _, _) = diverged_run();
    let wt = p.worktree();
    std::fs::write(wt.join("loose.txt"), "x\n").unwrap();
    let got = push_text(&p);
    assert!(got.starts_with("push=refused reason=dirty "), "{got}");
    git(&wt, &["add", "."]);
    git(&wt, &["commit", "-qm", "outside"]);
    let got = push_text(&p);
    assert!(got.starts_with("push=refused reason=sha "), "{got}");
}

/// GW-R-010, S6 — a branch missing from the remote is a normal first push.
#[test]
fn it_first_push_runs_the_gauntlet() {
    let (p, first) = pushed_run();
    assert!(first.starts_with("push=done sha="), "{first}");
    assert!(first.contains(" gauntlet=pass pr-body=opened"), "{first}");
    assert!(!first.contains("cached"), "{first}");
    git(
        &p.root,
        &["ls-remote", "--exit-code", "origin", "refs/heads/feat/demo"],
    );
}

/// GW-R-011, S7 — a check that cannot complete lets the gauntlet and the push go on.
#[test]
fn it_unreadable_remote_runs_the_gauntlet_and_the_push() {
    let (p, _) = pushed_run();
    p.land("s1", "src/client.rs");
    let gone = p.root.parent().unwrap().join("gone.git");
    git(
        &p.root,
        &["remote", "set-url", "origin", gone.to_str().unwrap()],
    );
    let log = p.run_dir().join("gauntlet.log");
    let _ = std::fs::remove_file(&log);
    let got = push_text(&p);
    assert!(got.starts_with("push=refused reason=push "), "{got}");
    assert!(log.is_file());
}

/// GW-R-012, S8 — `--no-run` applies the same check and gives the same fields.
#[test]
fn it_push_no_run_refuses_a_diverged_branch() {
    let p = Project::new();
    git(&p.root, &["checkout", "-q", "-b", "feat/x"]);
    git(&p.root, &["push", "-q", "-u", "origin", "feat/x"]);
    let remote = p.diverge_branch("feat/x");
    std::fs::write(p.root.join("mine.txt"), "m\n").unwrap();
    git(&p.root, &["add", "."]);
    git(&p.root, &["commit", "-qm", "mine"]);
    let local = git(&p.root, &["rev-parse", "--short", "HEAD"]);
    let got = p.ferrule(&["pr", "push", "--no-run"]).text();
    let line = format!("push=refused reason=diverged local={local} remote={remote} hint=");
    assert!(got.starts_with(&line), "{got}");
    assert!(!got.contains("reason=push"), "{got}");
    assert_eq!(p.origin_head("feat/x"), remote);
}

/// GW-E-001, GW-E-006 — a remote that moves after the check fails at the push, and the retry is diverged.
#[cfg(unix)]
#[test]
fn it_remote_moving_after_the_check_fails_at_the_push() {
    use std::os::unix::fs::PermissionsExt;
    let (p, _) = pushed_run();
    p.land("s1", "src/client.rs");
    let sibling = p.sibling("mover", "feat/demo");
    let hook = p.root.join(".git/hooks/pre-push");
    std::fs::write(
        &hook,
        format!(
            "#!/bin/sh\nunset GIT_DIR GIT_WORK_TREE\ncd {dir} || exit 1\necho m >> moved.txt\ngit add moved.txt\ngit -c user.name=t -c user.email=t@t commit -qm moved\ngit push -q origin feat/demo\n",
            dir = sibling.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    let got = push_text(&p);
    assert!(got.starts_with("push=refused reason=push "), "{got}");
    let retry = push_text(&p);
    assert!(
        retry.starts_with("push=refused reason=diverged "),
        "{retry}"
    );
}

/// Merges the origin's branch into the worktree with raw git.
fn merge_outside(p: &Project) {
    let wt = p.worktree();
    git(&wt, &["fetch", "-q", "origin"]);
    git(&wt, &["merge", "-q", "--no-edit", "origin/feat/demo"]);
}

/// GW-E-002 — after an outside merge the push refuses with `reason=sha` until the next stage commit.
#[test]
fn it_outside_merge_refuses_until_the_next_stage_commit() {
    let (p, _, _) = diverged_run();
    merge_outside(&p);
    let got = push_text(&p);
    assert!(got.starts_with("push=refused reason=sha "), "{got}");
    std::fs::write(p.worktree().join("more.txt"), "more\n").unwrap();
    git(&p.worktree(), &["add", "more.txt"]);
    let done = p.in_wt(&["stage", "commit", "demo.s1", "-m", "more"]);
    assert!(done.starts_with("commit=done sha="), "{done}");
    let pushed = push_text(&p);
    assert!(pushed.starts_with("push=done sha="), "{pushed}");
}

/// GW-E-005 — with nothing left to commit the run stays refused.
#[test]
fn it_outside_merge_with_nothing_left_stays_refused() {
    let (p, _, _) = diverged_run();
    merge_outside(&p);
    let done = p.in_wt(&["stage", "commit", "demo.s1", "-m", "more"]);
    assert!(
        done.starts_with("commit=refused reason=nothing-staged "),
        "{done}"
    );
    let got = push_text(&p);
    assert!(got.starts_with("push=refused reason=sha "), "{got}");
}

#[test]
fn it_pr_open_adopts_the_branch_pr_a_crash_left() {
    let p = Project::new();
    let gh = &p.gh;
    p.ok(&["run", "init", "demo"]);
    p.artifacts();
    seed_set(&p.root, "demo", &["issue=12", "branch=feat/demo"]);
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
    serve(gh, &pr_list("feat/demo"), "[]");
    assert_eq!(p.ok(&["pr", "open", "demo"]), "pr=34 state=draft");
    // A crash before the card recorded it: the PR exists, the card has
    // none. The rerun adopts the branch's open PR and opens none.
    p.ok(&["board", "set", "demo", "pr="]);
    serve(gh, &pr_list("feat/demo"), r#"[{"number":34}]"#);
    gh.serve(
        &["pr", "view", "34", "--json", "state,isDraft,body"],
        r#"{"state":"OPEN","isDraft":true,"body":"stale"}"#,
    );
    gh.serve(&["pr", "edit", "34", "--body-file", "-"], "");
    assert_eq!(p.ok(&["pr", "open", "demo"]), "pr=34 state=draft");
    assert_eq!(p.field("demo", "pr"), "34");
    let log = gh.log();
    assert_eq!(log.matches("pr create").count(), 1, "{log}");
    assert!(log.contains("pr edit 34 --body-file -"), "{log}");
    let shown = p.ok(&["board", "show", "demo"]);
    assert_eq!(shown.matches("pr=draft").count(), 1, "{shown}");
}

#[test]
fn it_issue_post_retries_post_only_what_is_missing() {
    let p = Project::new();
    let gh = &p.gh;
    p.ok(&["run", "init", "demo"]);
    p.artifacts();
    seed_set(&p.root, "demo", &["issue=12", "kind=slice", "epic=70"]);

    // A title with qualifier-like words: searched as one phrase, adopted
    // only on an exact match.
    let title = "Add -v and is:closed modes";
    std::fs::write(
        p.run_dir().join("follow-up-2.md"),
        format!("# {title}\n\n## Goal\nx\n"),
    )
    .unwrap();
    serve(
        gh,
        &find(title),
        &format!(r#"[{{"number":50,"title":"{title} too"}},{{"number":51,"title":"{title}"}}]"#),
    );
    serve(gh, &edit_issue("51", title), "");
    adopted_issue(gh, "51");
    assert_eq!(
        p.ok(&["issue", "post", "demo", "--follow-up", "2"]),
        "issue=51 action=adopted"
    );
    assert_eq!(
        p.ok(&["issue", "post", "demo", "--follow-up", "2"]),
        "issue=51 action=recorded"
    );
    assert_eq!(gh.log().matches("issue list").count(), 1);

    // The epic's comment fails after the issue's: the rerun posts only the
    // epic's, a third run nothing.
    std::fs::write(
        p.run_dir().join("issue-comment.md"),
        "MB-R-161 now retries forever.\n",
    )
    .unwrap();
    gh.serve(&["issue", "comment", "12", "--body-file", "-"], "");
    no_comments(gh, "12");
    no_comments(gh, "70");
    let failed = p.ferrule(&["issue", "post", "demo", "--comment"]).text();
    assert!(failed.ends_with("exit=1"), "{failed}");
    gh.serve(&["issue", "comment", "70", "--body-file", "-"], "");
    assert_eq!(
        p.ok(&["issue", "post", "demo", "--comment"]),
        "issue=12 action=commented"
    );
    assert_eq!(
        p.ok(&["issue", "post", "demo", "--comment"]),
        "issue=12 action=recorded"
    );
    let log = gh.log();
    assert_eq!(log.matches("issue comment 12").count(), 1, "{log}");
    assert_eq!(log.matches("issue comment 70").count(), 2, "{log}");
    // A new comment text is posted again, to both.
    std::fs::write(p.run_dir().join("issue-comment.md"), "MB-R-162 too.\n").unwrap();
    assert_eq!(
        p.ok(&["issue", "post", "demo", "--comment"]),
        "issue=12 action=commented"
    );
    assert_eq!(gh.log().matches("issue comment 12").count(), 2);
}

const BLOCK: &str = "mutation($issue: ID!, $other: ID!) { addBlockedBy(input: {issueId: $issue, blockingIssueId: $other}) { clientMutationId } }";
const MILESTONES: &str = "repos/{owner}/{repo}/milestones";

/// The `lb` epic run with its artifacts and `fields` on the card.
fn epic_project(fields: &[&str]) -> Project {
    let p = Project::new();
    p.ok(&["run", "init", "lb"]);
    let dir = p.root.join(".ferrule/runs/lb");
    for e in std::fs::read_dir(fixtures().join("board/artifacts/lb")).unwrap() {
        let e = e.unwrap();
        std::fs::copy(e.path(), dir.join(e.file_name())).unwrap();
    }
    let mut all = vec!["kind=epic"];
    all.extend_from_slice(fields);
    seed_set(&p.root, "lb", &all);
    p
}

fn milestone_json(n: u64, title: &str, description: &str) -> String {
    format!(
        r#"{{"number":{n},"title":"{title}","description":{},"state":"open"}}"#,
        serde_json::json!(description)
    )
}

const EPIC: &str = "Load balancing core";

fn serve_list_all(gh: &FakeGh, json: &str) {
    gh.serve(
        &[
            "api",
            "--paginate",
            "--slurp",
            &format!("{MILESTONES}?state=all&per_page=100"),
        ],
        json,
    );
}

fn create_milestone_args() -> [&'static str; 6] {
    ["api", "-X", "POST", MILESTONES, "--input", "-"]
}

fn serve_get_milestone(gh: &FakeGh, n: u64) {
    gh.serve(
        &["api", &format!("{MILESTONES}/{n}")],
        &milestone_json(n, EPIC, "brief"),
    );
}

fn serve_milestone_issues(gh: &FakeGh, open: &str) {
    gh.serve(
        &[
            "api",
            "--paginate",
            "--slurp",
            "repos/{owner}/{repo}/issues?milestone=4&state=open&per_page=100",
        ],
        open,
    );
}

/// The create call of slice issue `title`, answering issue `n`.
fn serve_slice_create(gh: &FakeGh, title: &str, n: u64) {
    gh.serve(
        &[
            "issue",
            "create",
            "--title",
            title,
            "--body-file",
            "-",
            "--milestone",
            EPIC,
        ],
        &format!("https://github.com/o/r/issues/{n}\n"),
    );
}

/// The call making `issue` blocked by `blocker`.
fn block_args(issue: u64, blocker: u64) -> Vec<String> {
    vec![
        "api".into(),
        "graphql".into(),
        "-f".into(),
        format!("issue=I{issue}"),
        "-f".into(),
        format!("other=I{blocker}"),
        "-f".into(),
        format!("query={BLOCK}"),
    ]
}

fn serve_block(gh: &FakeGh, issue: u64, blocker: u64) {
    serve(gh, &block_args(issue, blocker), "{}");
}

/// S1, RU-R-013, RU-R-016, RU-R-023 — an epic files a milestone and no issue; its slices go
/// inside it, the second blocked by the first.
#[test]
fn it_epic_files_a_milestone_and_its_slices() {
    let p = epic_project(&[]);
    let gh = &p.gh;
    serve_list_all(gh, "[[]]");
    gh.serve(&create_milestone_args(), &milestone_json(4, EPIC, "brief"));
    assert_eq!(p.ok(&["issue", "post", "lb"]), "milestone=4 action=created");
    assert_eq!(p.field("lb", "milestone"), "4");
    assert_eq!(p.field("lb", "issue"), "");
    assert!(!gh.log().contains("issue create"), "{}", gh.log());

    serve_get_milestone(gh, 4);
    serve_milestone_issues(gh, "[[]]");
    serve_slice_create(gh, "Walking skeleton", 71);
    serve_slice_create(gh, "Batteries", 72);
    serve_graph(gh, "71", &[], &[]);
    serve_graph(gh, "72", &[], &[]);
    serve_block(gh, 72, 71);
    assert_eq!(
        p.ok(&["issue", "post", "lb", "--slice", "1"]),
        "issue=71 action=created"
    );
    assert_eq!(
        p.ok(&["issue", "post", "lb", "--slice", "2"]),
        "issue=72 action=created"
    );
    assert_eq!(p.field("lb", "slices"), "[71, 72]");
    let log = gh.log();
    assert!(log.contains("--milestone Load balancing core"), "{log}");
    assert!(log.contains("issue=I72 -f other=I71"), "{log}");
}

/// S2, RU-R-019 — an epic on an existing issue closes it with a link to the milestone, edits
/// nothing and keeps the issue on the card.
#[test]
fn it_epic_on_an_issue_closes_it_with_a_link() {
    let p = epic_project(&["named-issue=7"]);
    let gh = &p.gh;
    serve_list_all(gh, "[[]]");
    gh.serve(&create_milestone_args(), &milestone_json(4, EPIC, "brief"));
    serve_graph(gh, "7", &[], &[]);
    no_comments(gh, "7");
    gh.serve(
        &[
            "issue",
            "close",
            "7",
            "--comment",
            "Moved to https://github.com/o/r/milestone/4.\n",
        ],
        "",
    );
    assert_eq!(p.ok(&["issue", "post", "lb"]), "milestone=4 action=created");
    assert_eq!(p.field("lb", "issue"), "7");
    let log = gh.log();
    assert!(
        log.contains("issue close 7 --comment Moved to https://github.com/o/r/milestone/4."),
        "{log}"
    );
    assert!(!log.contains("issue edit"), "{log}");
}

/// S9, RU-R-030 — a slice whose blocker write failed is found in the milestone on the rerun,
/// which writes the blocker and files no second issue.
#[test]
fn it_epic_rerun_after_a_failed_relationship_adds_only_it() {
    let p = epic_project(&["milestone=4"]);
    let gh = &p.gh;
    serve_get_milestone(gh, 4);
    serve_milestone_issues(gh, "[[]]");
    serve_slice_create(gh, "Walking skeleton", 71);
    serve_slice_create(gh, "Batteries", 72);
    serve_graph(gh, "71", &[], &[]);
    serve_graph(gh, "72", &[], &[]);
    p.ok(&["issue", "post", "lb", "--slice", "1"]);
    let block = block_args(72, 71);
    let block: Vec<&str> = block.iter().map(String::as_str).collect();
    gh.serve_err(&block, "boom", 1);
    let failed = p.ferrule(&["issue", "post", "lb", "--slice", "2"]).text();
    assert!(failed.ends_with("exit=1"), "{failed}");
    assert_eq!(
        p.field("lb", "slices"),
        "[71]",
        "the slot waits for the blocker"
    );
    // Issue 72 is in the milestone now, its text not the run's.
    serve_milestone_issues(gh, r#"[[{"number":72,"title":"Batteries"}]]"#);
    gh.clear(&block);
    gh.serve(&block, "{}");
    let text = p.ok(&["issue", "body", "lb", "--slice", "2"]);
    let (_, body) = text.split_once("\n\n").unwrap();
    gh.serve(
        &["issue", "view", "72", "--json", GH_FIELDS],
        &gh_json(&format!(
            "#72 Batteries [OPEN]\nhttps://github.com/o/r/issues/72\nby someone\n\n{body}\n"
        )),
    );
    assert_eq!(
        p.ok(&["issue", "post", "lb", "--slice", "2"]),
        "issue=72 action=adopted"
    );
    assert_eq!(p.field("lb", "slices"), "[71, 72]");
    let log = gh.log();
    assert_eq!(
        log.matches("issue create --title Batteries").count(),
        1,
        "{log}"
    );
    assert_eq!(log.matches("issue=I72 -f other=I71").count(), 2, "{log}");
    assert!(!log.contains("issue edit"), "the text is the run's: {log}");
}

/// RU-E-006 — a closed milestone of the epic's title refuses, writing nothing.
#[test]
fn it_epic_refuses_a_closed_milestone() {
    let p = epic_project(&[]);
    let gh = &p.gh;
    let closed = milestone_json(4, EPIC, "x").replace("\"open\"", "\"closed\"");
    serve_list_all(gh, &format!("[[{closed}]]"));
    let got = p.ferrule(&["issue", "post", "lb"]).text();
    assert!(
        got.starts_with("post=refused reason=milestone-closed milestone=4 hint="),
        "{got}"
    );
    assert!(got.ends_with("exit=1"), "{got}");
    assert_eq!(p.field("lb", "milestone"), "");
}

/// RU-E-010 — a description GitHub answers with a 422 refuses with its length, on a create
/// and on an update.
#[test]
fn it_epic_refuses_a_description_the_host_rejects() {
    let p = epic_project(&[]);
    let gh = &p.gh;
    serve_list_all(gh, "[[]]");
    gh.serve_err(
        &create_milestone_args(),
        "gh: Validation Failed (HTTP 422)",
        1,
    );
    let got = p.ferrule(&["issue", "post", "lb"]).text();
    assert!(
        got.starts_with("post=refused reason=rejected length="),
        "{got}"
    );
    assert!(got.contains("HTTP 422"), "{got}");
    assert!(got.contains("hint="), "{got}");
    assert!(got.ends_with("exit=1"), "{got}");
    serve_list_all(gh, &format!("[[{}]]", milestone_json(4, EPIC, "old")));
    gh.serve_err(
        &[
            "api",
            "-X",
            "PATCH",
            &format!("{MILESTONES}/4"),
            "--input",
            "-",
        ],
        "gh: Validation Failed (HTTP 422)",
        1,
    );
    let got = p.ferrule(&["issue", "post", "lb"]).text();
    assert!(
        got.starts_with("post=refused reason=rejected length="),
        "{got}"
    );
}

/// RU-R-022 — slices keep the position they were filed at; a slice whose dependency is not
/// filed refuses and writes nothing.
#[test]
fn it_slices_keep_their_position() {
    let p = epic_project(&["milestone=4"]);
    let gh = &p.gh;
    let slices = p.root.join(".ferrule/runs/lb/slices.md");
    let text = std::fs::read_to_string(&slices).unwrap();
    std::fs::write(
        &slices,
        text.replace("Depends on: 1\n", "Depends on: none\n"),
    )
    .unwrap();
    serve_get_milestone(gh, 4);
    serve_milestone_issues(gh, "[[]]");
    serve_slice_create(gh, "Walking skeleton", 71);
    serve_slice_create(gh, "Batteries", 72);
    serve_graph(gh, "71", &[], &[]);
    serve_graph(gh, "72", &[], &[]);
    serve_block(gh, 72, 71);
    let early = p.ferrule(&["issue", "post", "lb", "--slice", "3"]).text();
    assert!(
        early.starts_with("post=refused reason=no-slice slice=1 "),
        "{early}"
    );
    assert_eq!(p.field("lb", "slices"), "");
    assert_eq!(
        p.ok(&["issue", "post", "lb", "--slice", "2"]),
        "issue=72 action=created"
    );
    assert_eq!(p.field("lb", "slices"), "[-, 72]");
    assert_eq!(
        p.ok(&["issue", "post", "lb", "--slice", "1"]),
        "issue=71 action=created"
    );
    assert_eq!(p.field("lb", "slices"), "[71, 72]");
    assert_eq!(
        p.ok(&["issue", "post", "lb", "--slice", "2"]),
        "issue=72 action=recorded"
    );
    let epic = p.ok(&["issue", "body", "lb"]);
    assert!(!epic.contains("## Slices"), "{epic}");
    let body3 = p.ok(&["issue", "body", "lb", "--slice", "3"]);
    assert!(!body3.contains("Depends on"), "{body3}");
    assert!(body3.starts_with("# Nested groups\n"), "{body3}");
    let zero = p.ferrule(&["issue", "post", "lb", "--slice", "0"]).output();
    assert_eq!(zero.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&zero.stdout).starts_with("post=refused reason=no-slice slice=0 "),
        "{}",
        String::from_utf8_lossy(&zero.stdout)
    );
    assert_eq!(p.field("lb", "slices"), "[71, 72]");
}

/// The command lists the open milestones, one `#<n> <title>` each.
#[test]
fn it_issue_milestones_lists_open() {
    let p = Project::new();
    p.gh.serve(
        &[
            "api",
            "--paginate",
            "--slurp",
            &format!("{MILESTONES}?state=open&per_page=100"),
        ],
        &format!(
            "[[{},{}]]",
            milestone_json(4, EPIC, "a"),
            milestone_json(5, "Load balancing edge", "b")
        ),
    );
    assert_eq!(
        p.ok(&["issue", "milestones"]),
        "#4 Load balancing core\n#5 Load balancing edge"
    );
}

/// `--view <n>` prints the milestone's description.
#[test]
fn it_issue_milestones_view_prints_the_description() {
    let p = Project::new();
    serve_get_milestone(&p.gh, 4);
    assert_eq!(p.ok(&["issue", "milestones", "--view", "4"]), "brief");
}

/// A tracker without milestones gets the prose form of a body, and a config that does not
/// load is no reason to fail the read.
#[test]
fn it_issue_body_is_prose_without_milestones() {
    let p = epic_project(&[]);
    let settings = p.root.join(".ferrule/ferrule.toml");
    let kept = std::fs::read_to_string(&settings).unwrap();
    std::fs::write(&settings, "[backend]\ntracker = \"jira\"\n").unwrap();
    seed_set(&p.root, "lb", &["slices=[71, 72]"]);
    let got = p.ok(&["issue", "body", "lb", "--slice", "2"]);
    assert!(
        got.starts_with("# Load balancing core: Batteries\n"),
        "{got}"
    );
    assert!(got.contains("**Part of epic**"), "{got}");
    std::fs::write(&settings, "[backend\n").unwrap();
    let got = p.ferrule(&["issue", "body", "lb", "--slice", "2"]).text();
    assert!(got.ends_with("exit=0"), "{got}");
    std::fs::write(&settings, kept).unwrap();
}

#[test]
fn it_a_mistyped_stop_or_follow_up_list_is_a_usage_error() {
    let p = Project::new();
    for args in [
        &["gate", "approve", "demo", "gate2", "--stops", "s1,3"][..],
        &["stage", "replan", "demo", "approve", "--stops", "s1;s2"],
        &["pr", "publish", "demo", "--follow-ups", "1,x"],
        &["pr", "publish", "demo", "--follow-ups", ""],
    ] {
        let got = p.ferrule(args).text();
        assert!(got.ends_with("exit=2"), "{args:?}: {got}");
    }
}

/// GW-R-039, GW-R-042 — each logged commit is mapped onto the rebased one at its position.
#[test]
fn it_rebase_maps_each_logged_commit_onto_the_moved_base() {
    let p = pushed_s1();
    let wt = p.worktree();
    std::fs::write(wt.join("src/client.rs"), "fix\n").unwrap();
    git(&wt, &["add", "."]);
    git(&wt, &["commit", "-qm", "fix"]);
    let fix = wt_head(&p);
    seed_log(
        &p.root,
        "2026-10-05T09:00",
        "demo.s1",
        &[&format!("commit={fix}")],
    );
    p.diverge_branch("main");
    git(&wt, &["fetch", "-q", "origin", "main"]);
    let got = rebase(&p, &["--onto", "origin/main"]);
    let head = wt_head(&p);
    assert!(
        got.starts_with(&format!("rebase=done head={head} mapped=3\n")),
        "{got}"
    );
    let new: Vec<String> = git(
        &wt,
        &[
            "rev-list",
            "--reverse",
            "--abbrev-commit",
            "origin/main..HEAD",
        ],
    )
    .lines()
    .map(|l| l[..7].to_owned())
    .collect();
    assert_eq!(new.len(), 3, "{new:?}");
    let s0 = p.ok(&["board", "show", "demo.s0"]);
    assert!(s0.contains(&format!("commit={}", new[0])), "{s0}");
    let s1 = p.ok(&["board", "show", "demo.s1"]);
    let at = |sha: &str| s1.find(&format!("commit={sha}")).expect(&s1);
    assert!(at(&new[1]) < at(&new[2]), "{s1}");
    assert!(s1.contains(&format!("commit={fix}")), "{s1}");
    let run = p.ok(&["board", "show", "demo"]);
    assert!(
        run.contains(&format!("rebase={head} base=origin/main from={fix}")),
        "{run}"
    );
}

/// GW-R-056 — without a base the main branch is fetched from origin.
#[test]
fn it_rebase_without_onto_fetches_the_main_branch() {
    let p = pushed_s1();
    p.diverge_branch("main");
    let got = rebase(&p, &[]);
    assert!(got.starts_with("rebase=done "), "{got}");
    git(
        &p.worktree(),
        &["merge-base", "--is-ancestor", "origin/main", "HEAD"],
    );
}

/// GW-R-018, GW-R-049 — a mapped rebase reconciles clean and shows no diverged row.
#[test]
fn it_rebased_run_reconciles_clean() {
    let p = pushed_s1();
    p.diverge_branch("main");
    let got = rebase(&p, &[]);
    assert!(got.starts_with("rebase=done "), "{got}");
    serve_host(&p);
    let out = p.ferrule(&["run", "reconcile", "demo"]).text();
    assert!(!out.contains("row=diverged"), "{out}");
    assert!(!out.contains("row=gauntlet"), "{out}");
    assert_eq!(
        out,
        "differ row=pr #34 body behind the board\nreconcile=differ n=1 next=s2\nexit=1"
    );
    let local = p.ferrule(&["run", "reconcile", "demo", "--local"]).text();
    assert!(local.contains("reconcile=clean"), "{local}");
}

/// GW-R-040 — an untracked file refuses the rebase.
#[test]
fn it_rebase_refuses_a_dirty_worktree() {
    let p = pushed_s1();
    p.diverge_branch("main");
    let before = wt_head(&p);
    std::fs::write(p.worktree().join("loose.txt"), "x\n").unwrap();
    let got = rebase(&p, &[]);
    assert!(
        got.starts_with("rebase=refused reason=dirty hint="),
        "{got}"
    );
    assert_eq!(wt_head(&p), before);
}

/// GW-R-053 — a stage in progress refuses the rebase.
#[test]
fn it_rebase_refuses_with_a_stage_in_progress() {
    let p = pushed_s1();
    p.diverge_branch("main");
    let before = wt_head(&p);
    p.ok(&["board", "move", "demo.s1", "inprogress"]);
    let got = rebase(&p, &[]);
    assert!(
        got.starts_with("rebase=refused reason=state card=demo.s1 hint="),
        "{got}"
    );
    assert_eq!(wt_head(&p), before);
}

/// GW-R-041, GW-E-013 — a conflict stops the rebase; the resolved one is mapped on the next run.
#[test]
fn it_rebase_stops_on_a_conflict_and_maps_after_continue() {
    let p = pushed_s1();
    conflicted(&p);
    let wt = p.worktree();
    std::fs::write(wt.join("src/client.rs"), "resolved\n").unwrap();
    git(&wt, &["add", "src/client.rs"]);
    let cont = git_status(&wt, &["-c", "core.editor=true", "rebase", "--continue"]);
    assert!(cont.status.success(), "{cont:?}");
    let got = rebase(&p, &[]);
    assert!(
        got.starts_with(&format!("rebase=done head={} mapped=2\n", wt_head(&p))),
        "{got}"
    );
}

/// Whether `ferrule hook shell` lets `cmd` run from the run's worktree.
fn hook_allows(p: &Project, cmd: &str) -> bool {
    use std::io::Write;
    use std::process::Stdio;
    let wt = p.worktree();
    let payload = serde_json::json!({
        "tool_name": "Bash",
        "tool_input": {"command": cmd, "description": "d"},
        "cwd": wt,
    });
    let mut child = Command::new(env!("CARGO_BIN_EXE_ferrule"))
        .args(["hook", "shell"])
        .current_dir(&wt)
        .env("CLAUDE_PROJECT_DIR", &p.root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(payload.to_string().as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{cmd}");
    out.stdout.is_empty()
}

/// SG-R-006, SG-R-007, SG-R-008 — the guard opens staging and `--continue` only while a rebase is stopped.
#[test]
fn it_guard_lets_an_agent_finish_a_stopped_rebase() {
    let p = pushed_s1();
    conflicted(&p);
    assert!(hook_allows(&p, "git add src/client.rs"));
    assert!(hook_allows(&p, "GIT_EDITOR=true git rebase --continue"));
    assert!(!hook_allows(&p, "git commit -m x"));
    assert!(!hook_allows(&p, "git rebase origin/main"));
    let wt = p.worktree();
    std::fs::write(wt.join("src/client.rs"), "resolved\n").unwrap();
    git(&wt, &["add", "src/client.rs"]);
    let cont = git_status(&wt, &["-c", "core.editor=true", "rebase", "--continue"]);
    assert!(cont.status.success(), "{cont:?}");
    let got = rebase(&p, &[]);
    assert!(got.starts_with("rebase=done "), "{got}");
    assert!(!hook_allows(&p, "git rebase --continue"));
}

/// GW-R-057 — a rebase already in progress refuses another and logs nothing.
#[test]
fn it_rebase_refuses_while_a_rebase_is_in_progress() {
    let p = pushed_s1();
    conflicted(&p);
    let before = cards(&p);
    let got = rebase(&p, &[]);
    assert!(
        got.starts_with("rebase=refused reason=in-progress hint="),
        "{got}"
    );
    for want in [
        "git rebase --continue",
        "git rebase --abort",
        "ferrule run rebase demo",
    ] {
        assert!(got.contains(want), "{want}: {got}");
    }
    assert_eq!(cards(&p), before);
}

/// GW-R-047 — an aborted rebase leaves the branch and the cards as before.
#[test]
fn it_aborted_rebase_logs_nothing() {
    let p = pushed_s1();
    let head = wt_head(&p);
    let before = cards(&p);
    conflicted(&p);
    let abort = git_status(&p.worktree(), &["rebase", "--abort"]);
    assert!(abort.status.success(), "{abort:?}");
    assert_eq!(wt_head(&p), head);
    assert_eq!(cards(&p), before);
}

/// GW-R-054 — rebasing again once mapped is unchanged and logs nothing.
#[test]
fn it_rebase_again_is_unchanged() {
    let p = pushed_s1();
    p.diverge_branch("main");
    let got = rebase(&p, &[]);
    assert!(got.starts_with("rebase=done "), "{got}");
    let before = cards(&p);
    let got = rebase(&p, &[]);
    assert!(
        got.starts_with(&format!("rebase=unchanged head={}\n", wt_head(&p))),
        "{got}"
    );
    assert_eq!(cards(&p), before);
}

/// GW-R-043, GW-E-014 — a dropped or reworded stage commit refuses the mapping.
#[test]
fn it_rebase_refuses_a_dropped_or_reworded_commit() {
    for reword in [false, true] {
        let p = pushed_s1();
        p.diverge_branch("main");
        let wt = p.worktree();
        git(&wt, &["fetch", "-q", "origin", "main"]);
        git(&wt, &["rebase", "-q", "origin/main"]);
        if reword {
            git(&wt, &["commit", "-q", "--amend", "-m", "changed"]);
        } else {
            git(&wt, &["reset", "-q", "--hard", "HEAD^"]);
        }
        let before = cards(&p);
        let got = rebase(&p, &[]);
        let want = if reword {
            "rebase=refused reason=mismatch at=2 "
        } else {
            "rebase=refused reason=count old=2 new=1 hint="
        };
        assert!(got.starts_with(want), "{got}");
        assert_eq!(cards(&p), before);
    }
}

/// GW-R-044 — only committed stages are mapped; a later stage commits on the rebased branch.
#[test]
fn it_rebase_mid_run_maps_only_committed_stages() {
    let (p, _) = pushed_run();
    p.diverge_branch("main");
    let got = rebase(&p, &[]);
    assert!(got.contains("mapped=1\n"), "{got}");
    let head = wt_head(&p);
    p.ok(&["board", "move", "demo.s1", "inprogress"]);
    let wt = p.worktree();
    std::fs::create_dir_all(wt.join("src")).unwrap();
    std::fs::write(wt.join("src/client.rs"), "s1\n").unwrap();
    git(&wt, &["add", "."]);
    p.ok(&["board", "move", "demo.s1", "inreview"]);
    let done = p.in_wt(&["stage", "commit", "demo.s1", "-m", "s1"]);
    let sha = wt_head(&p);
    assert!(
        done.starts_with(&format!("commit=done sha={sha} card=demo.s1")),
        "{done}"
    );
    assert_eq!(git(&wt, &["rev-parse", "--short", "HEAD^"]), head);
    let verify = p.ferrule(&["stage", "verify", "demo", "s1", &sha]).text();
    assert_eq!(verify, "sha=ok\nexit=0");
}

/// GW-R-046 — the approved reviews and the done stages stay after a rebase.
#[test]
fn it_rebase_keeps_the_approved_reviews() {
    let p = pushed_s1();
    seed_log(
        &p.root,
        "2026-10-05T09:00",
        "demo",
        &["review=approved", "scope=s1"],
    );
    p.diverge_branch("main");
    let got = rebase(&p, &[]);
    assert!(got.starts_with("rebase=done "), "{got}");
    let run = p.ok(&["board", "show", "demo"]);
    assert!(run.contains("review=approved scope=s1"), "{run}");
    assert!(p.ok(&["board", "show", "demo.s1"]).contains("done"));
}

/// A pushed run with its main branch moved and rebased through ferrule: the
/// project and the head pushed before the rebase.
fn rebased_after_push() -> (Project, String) {
    let p = pushed_s1();
    let pushed = wt_head(&p);
    p.diverge_branch("main");
    let got = rebase(&p, &[]);
    assert!(got.starts_with("rebase=done "), "{got}");
    (p, pushed)
}

/// GW-R-007, GW-R-045, GW-R-050 — a push after a rebase reruns the gauntlet and forces with a lease.
#[test]
fn it_push_after_a_rebase_reruns_the_gauntlet_and_forces_with_lease() {
    let (p, _) = rebased_after_push();
    let head = wt_head(&p);
    let log = p.run_dir().join("gauntlet.log");
    let _ = std::fs::remove_file(&log);
    let got = push_text(&p);
    assert!(
        got.starts_with(&format!("push=done sha={head} gauntlet=pass forced")),
        "{got}"
    );
    assert!(!got.contains("cached"), "{got}");
    assert!(log.is_file());
    assert_eq!(p.origin_head("feat/demo"), head);
    let again = push_text(&p);
    assert!(again.starts_with("push=done "), "{again}");
    assert!(!again.contains("forced"), "{again}");
}

/// GW-R-045 — a push after a recorded rebase reruns the gauntlet even when the tree's pass is cached.
#[test]
fn it_push_after_a_rebase_reruns_a_cached_gauntlet() {
    let p = pushed_s1();
    let head = wt_head(&p);
    seed_log(
        &p.root,
        "2026-10-05T09:00",
        "demo",
        &[&format!("rebase={head} base=origin/main from={head}")],
    );
    let log = p.run_dir().join("gauntlet.log");
    let _ = std::fs::remove_file(&log);
    let got = push_text(&p);
    assert!(
        got.starts_with(&format!("push=done sha={head} gauntlet=pass forced")),
        "{got}"
    );
    assert!(!got.contains("cached"), "{got}");
    assert!(log.is_file());
}

/// GW-R-064 — after a mapped rebase the status shows no gauntlet result until a later push logs one.
#[test]
fn it_run_status_shows_no_gauntlet_result_from_before_a_rebase() {
    let p = pushed_s1();
    let has = |p: &Project, line: &str| {
        p.ok(&["run", "status", "demo"])
            .lines()
            .any(|l| l.starts_with(line))
    };
    assert!(has(&p, "gauntlet=pass"));
    p.diverge_branch("main");
    let got = rebase(&p, &[]);
    assert!(got.starts_with("rebase=done"), "{got}");
    let st = p.ok(&["run", "status", "demo"]);
    assert!(st.lines().any(|l| l == "gauntlet=-"), "{st}");
    let got = push_text(&p);
    assert!(got.starts_with("push=done"), "{got}");
    assert!(has(&p, "gauntlet=pass"));
}

/// GW-R-051 — a remote moved by someone else refuses the force-push before the gauntlet.
#[test]
fn it_push_after_a_rebase_refuses_a_remote_moved_by_someone_else() {
    let (p, pushed) = rebased_after_push();
    let dir = p.sibling("someone", "feat/demo");
    std::fs::write(dir.join("theirs.txt"), "x\n").unwrap();
    git(&dir, &["add", "."]);
    git(&dir, &["commit", "-qm", "theirs"]);
    git(&dir, &["push", "-q", "origin", "feat/demo"]);
    let other = p.origin_head("feat/demo");
    let log = p.run_dir().join("gauntlet.log");
    let _ = std::fs::remove_file(&log);
    let got = push_text(&p);
    assert!(
        got.starts_with(&format!(
            "push=refused reason=lease remote={other} pushed={pushed} hint="
        )),
        "{got}"
    );
    assert_eq!(p.origin_head("feat/demo"), other);
    assert!(!log.exists());
}

/// GW-R-051 — a branch deleted on origin refuses the force-push before the gauntlet.
#[test]
fn it_push_after_a_rebase_refuses_a_branch_deleted_on_origin() {
    let (p, pushed) = rebased_after_push();
    git(&p.origin(), &["branch", "-D", "feat/demo"]);
    let log = p.run_dir().join("gauntlet.log");
    let _ = std::fs::remove_file(&log);
    let got = push_text(&p);
    assert!(
        got.starts_with(&format!(
            "push=refused reason=lease remote=none pushed={pushed} hint="
        )),
        "{got}"
    );
    assert!(!log.exists());
    let heads = git(&p.origin(), &["branch", "--list", "feat/demo"]);
    assert!(heads.is_empty(), "{heads}");
}

/// GW-R-062 — a remote check that cannot complete lets the gauntlet and the push go on.
#[test]
fn it_push_after_a_rebase_goes_on_when_the_remote_is_unreadable() {
    let (p, _) = rebased_after_push();
    let gone = p.root.parent().unwrap().join("gone.git");
    git(
        &p.root,
        &["remote", "set-url", "origin", gone.to_str().unwrap()],
    );
    let log = p.run_dir().join("gauntlet.log");
    let _ = std::fs::remove_file(&log);
    let got = push_text(&p);
    assert!(got.starts_with("push=refused reason=push "), "{got}");
    assert!(log.is_file());
}

/// GW-R-055 — `--no-run` never force-pushes.
#[test]
fn it_push_no_run_never_forces() {
    let (p, _) = rebased_after_push();
    let before = p.origin_head("feat/demo");
    let got = p.in_wt(&["pr", "push", "--no-run"]);
    assert!(got.starts_with("push=refused reason=diverged "), "{got}");
    assert_eq!(p.origin_head("feat/demo"), before);
}

/// GW-R-048, GW-R-061 — a rebase in progress refuses the push and the commit, adopted or not.
#[test]
fn it_push_and_commit_refuse_while_a_rebase_is_in_progress() {
    let p = pushed_s1();
    conflicted(&p);
    let got = push_text(&p);
    assert!(
        got.starts_with("push=refused reason=rebasing hint="),
        "{got}"
    );
    for args in [
        &["stage", "commit", "demo.s1", "-m", "x"][..],
        &["stage", "commit", "demo.s1", "--adopt", "HEAD"],
    ] {
        let got = p.in_wt(args);
        assert!(
            got.starts_with("commit=refused reason=rebasing hint="),
            "{args:?}: {got}"
        );
    }
}
