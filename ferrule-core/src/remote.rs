//! The tracker and CI hosts: issues, pull requests and failed CI runs,
//! through `gh` for GitHub and `curl` for Jira and Bitbucket. Everything is
//! returned as an [`Item`] so the commands above never see a host's JSON.

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};

use serde::Deserialize;
use serde_json::Value;

use crate::config::{Ci, Tracker};
use crate::store::DIR;

#[derive(Debug)]
pub enum Error {
    /// A program could not run or exited non-zero: its stderr.
    Command(String),
    /// A credentials file is missing or malformed.
    Credentials(String),
    /// The host answered with JSON of an unexpected shape.
    Shape(String),
    /// Nothing matched (no failed run, no failing job).
    NotFound(String),
    /// The configured host has no implementation of this write yet.
    Unsupported(&'static str),
    /// The number names a pull request, not an issue (GitHub serves both).
    PullRequest(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Command(e) | Error::Credentials(e) | Error::Shape(e) | Error::NotFound(e) => {
                f.write_str(e)
            }
            Error::Unsupported(what) => write!(f, "{what} is only implemented for GitHub yet"),
            Error::PullRequest(n) => {
                write!(
                    f,
                    "#{n} is a pull request, not an issue: ferrule pr view {n}"
                )
            }
        }
    }
}

impl std::error::Error for Error {}

/// Runs `program` with `args`, `stdin` fed in, and returns its stdout.
pub fn run(program: &str, args: &[&str], stdin: Option<&str>, cwd: &Path) -> Result<String, Error> {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if !cwd.as_os_str().is_empty() {
        cmd.current_dir(cwd);
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| Error::Command(format!("{program}: {e}")))?;
    if let (Some(input), Some(mut pipe)) = (stdin, child.stdin.take()) {
        pipe.write_all(input.as_bytes())
            .map_err(|e| Error::Command(format!("{program}: {e}")))?;
    }
    let out = child
        .wait_with_output()
        .map_err(|e| Error::Command(format!("{program}: {e}")))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_owned();
        return Err(Error::Command(if err.is_empty() {
            format!("{program} {} failed", args.first().unwrap_or(&""))
        } else {
            err
        }));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn json<T: for<'de> Deserialize<'de>>(text: &str, what: &str) -> Result<T, Error> {
    serde_json::from_str(text).map_err(|e| Error::Shape(format!("{what}: {e}")))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comment {
    pub author: String,
    pub at: String,
    pub body: String,
}

/// An issue or pull request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    /// `#7 Title [OPEN]` or `KEY-1 Summary [In Progress]`.
    pub head: String,
    pub url: Option<String>,
    pub by: String,
    pub body: String,
    pub comments: Vec<Comment>,
}

/// The parts of an item `issue view` and `pr view` print.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Select {
    /// The body; with neither `body` nor `comments`, both print.
    pub body: bool,
    pub comments: bool,
    /// Only the latest n comments.
    pub last: Option<usize>,
    /// Only the comments by this author, ignoring case.
    pub author: Option<String>,
}

impl Select {
    #[must_use]
    pub fn shows_body(&self) -> bool {
        self.body || !self.comments
    }

    #[must_use]
    pub fn shows_comments(&self) -> bool {
        self.comments || !self.body
    }

    /// The comments of `item` this selection keeps, oldest first.
    #[must_use]
    pub fn comments<'a>(&self, item: &'a Item) -> Vec<&'a Comment> {
        let mut kept: Vec<&Comment> = item
            .comments
            .iter()
            .filter(|c| {
                self.author
                    .as_ref()
                    .is_none_or(|a| c.author.eq_ignore_ascii_case(a))
            })
            .collect();
        if let Some(n) = self.last {
            kept.drain(..kept.len().saturating_sub(n));
        }
        kept
    }

    /// `<n> comment(s)`, or `<k> of <n> comment(s)` when some were left out.
    #[must_use]
    pub fn count(&self, item: &Item) -> String {
        let (k, n) = (self.comments(item).len(), item.comments.len());
        if k == n {
            format!("{n} comment(s)")
        } else {
            format!("{k} of {n} comment(s)")
        }
    }
}

impl Item {
    /// Title, body and comments as compact plain text.
    #[must_use]
    pub fn text(&self) -> String {
        self.text_for(&Select::default())
    }

    /// The parts `select` keeps, as compact plain text. The head, URL and
    /// author always print.
    #[must_use]
    pub fn text_for(&self, select: &Select) -> String {
        let mut out = format!("{}\n", self.head);
        if let Some(url) = &self.url {
            let _ = writeln!(out, "{url}");
        }
        let _ = writeln!(out, "by {}\n", self.by);
        if select.shows_body() {
            let _ = writeln!(out, "{}\n", self.body);
        }
        if select.shows_comments() {
            let _ = writeln!(out, "--- {} ---", select.count(self));
            for c in select.comments(self) {
                let _ = writeln!(out, "\n[{} @ {}]\n{}", c.author, c.at, c.body);
            }
        }
        out
    }

    /// The title alone: the head without its number and state.
    #[must_use]
    pub fn title(&self) -> &str {
        let t = self.head.split_once(' ').map_or("", |(_, t)| t);
        match t.rfind(" [") {
            Some(i) if t.ends_with(']') => &t[..i],
            _ => t,
        }
    }
}

#[derive(Deserialize)]
struct GhAuthor {
    login: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhComment {
    author: Option<GhAuthor>,
    created_at: String,
    body: Option<String>,
}

#[derive(Deserialize)]
struct GhItem {
    number: u64,
    title: String,
    state: String,
    url: String,
    author: Option<GhAuthor>,
    body: Option<String>,
    #[serde(default)]
    comments: Vec<GhComment>,
}

fn login(a: Option<&GhAuthor>) -> String {
    a.and_then(|a| a.login.clone())
        .unwrap_or_else(|| "unknown".to_owned())
}

fn non_empty(s: Option<&str>, empty: &str) -> String {
    s.filter(|b| !b.is_empty()).unwrap_or(empty).to_owned()
}

impl From<GhItem> for Item {
    fn from(g: GhItem) -> Self {
        Item {
            head: format!("#{} {} [{}]", g.number, g.title, g.state),
            url: Some(g.url),
            by: login(g.author.as_ref()),
            body: non_empty(g.body.as_deref(), "(no body)"),
            comments: g
                .comments
                .iter()
                .map(|c| Comment {
                    author: login(c.author.as_ref()),
                    at: c.created_at.clone(),
                    body: non_empty(c.body.as_deref(), "(empty)"),
                })
                .collect(),
        }
    }
}

const GH_FIELDS: &str = "number,title,state,url,author,body,comments";

/// Every `text` leaf of an Atlassian document, joined by spaces.
fn adf_text(v: &Value) -> String {
    fn walk(v: &Value, out: &mut Vec<String>) {
        match v {
            Value::Object(m) => {
                if let Some(Value::String(t)) = m.get("text") {
                    out.push(t.clone());
                }
                for (k, x) in m {
                    if k != "text" {
                        walk(x, out);
                    }
                }
            }
            Value::Array(a) => a.iter().for_each(|x| walk(x, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    walk(v, &mut out);
    out.join(" ")
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct JiraCreds {
    base_url: String,
    email: String,
    api_token: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BitbucketCreds {
    workspace: String,
    repo_slug: String,
    username: String,
    app_password: String,
}

fn creds<T: for<'de> Deserialize<'de>>(root: &Path, file: &str) -> Result<T, Error> {
    let path = root.join(DIR).join(file);
    let text = std::fs::read_to_string(&path)
        .map_err(|e| Error::Credentials(format!("{}: {e}", path.display())))?;
    serde_json::from_str(&text).map_err(|e| Error::Credentials(format!("{}: {e}", path.display())))
}

/// `curl` with the credentials on stdin (`--config -`), never in argv.
fn curl(user: &str, pass: &str, args: &[&str], root: &Path) -> Result<String, Error> {
    let quote = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    let config = format!("user = \"{}:{}\"\n", quote(user), quote(pass));
    let mut all = vec!["-sf", "--config", "-"];
    all.extend_from_slice(args);
    run("curl", &all, Some(&config), root)
}

struct Jira {
    c: JiraCreds,
}

impl Jira {
    fn get(&self, path: &str, root: &Path) -> Result<Value, Error> {
        let url = format!("{}/rest/api/3/{path}", self.c.base_url);
        json(
            &curl(&self.c.email, &self.c.api_token, &[&url], root)?,
            "jira",
        )
    }

    fn issue(&self, key: &str, root: &Path) -> Result<Item, Error> {
        let i = self.get(
            &format!("issue/{key}?fields=summary,status,reporter,description"),
            root,
        )?;
        let c = self.get(&format!("issue/{key}/comment?orderBy=created"), root)?;
        let s = |v: &Value| v.as_str().unwrap_or("").to_owned();
        let f = &i["fields"];
        Ok(Item {
            head: format!(
                "{} {} [{}]",
                s(&i["key"]),
                s(&f["summary"]),
                s(&f["status"]["name"])
            ),
            url: None,
            by: f["reporter"]["displayName"]
                .as_str()
                .unwrap_or("unknown")
                .to_owned(),
            body: adf_text(&f["description"]),
            comments: c["comments"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .map(|c| Comment {
                            author: s(&c["author"]["displayName"]),
                            at: s(&c["created"]),
                            body: adf_text(&c["body"]),
                        })
                        .collect()
                })
                .unwrap_or_default(),
        })
    }
}

pub(crate) struct Bitbucket {
    c: BitbucketCreds,
}

impl Bitbucket {
    pub(crate) fn load(root: &Path) -> Result<Self, Error> {
        Ok(Bitbucket {
            c: creds(root, "bitbucket.local.json")?,
        })
    }

    pub(crate) fn api(&self, path: &str) -> String {
        format!(
            "https://api.bitbucket.org/2.0/repositories/{}/{}/{path}",
            self.c.workspace, self.c.repo_slug
        )
    }

    pub(crate) fn call(&self, args: &[&str], root: &Path) -> Result<String, Error> {
        curl(&self.c.username, &self.c.app_password, args, root)
    }

    fn pr(&self, n: &str, root: &Path) -> Result<Item, Error> {
        let api = self.api(&format!("pullrequests/{n}"));
        let pr: Value = json(&self.call(&[&api], root)?, "bitbucket")?;
        let comments_url = format!("{api}/comments?pagelen=50");
        let cs: Value = json(&self.call(&[&comments_url], root)?, "bitbucket")?;
        let s = |v: &Value, d: &str| v.as_str().unwrap_or(d).to_owned();
        let comments: Vec<Comment> = cs["values"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|c| c["deleted"] != Value::Bool(true))
            .map(|c| Comment {
                author: s(&c["user"]["display_name"], "unknown"),
                at: s(&c["created_on"], ""),
                body: s(&c["content"]["raw"], "(empty)"),
            })
            .collect();
        Ok(Item {
            head: format!(
                "#{} {} [{}]",
                pr["id"],
                s(&pr["title"], ""),
                s(&pr["state"], "")
            ),
            url: pr["links"]["html"]["href"].as_str().map(str::to_owned),
            by: s(&pr["author"]["display_name"], "unknown"),
            body: non_empty(pr["description"].as_str(), "(no body)"),
            comments,
        })
    }
}

/// An issue from the configured tracker: a GitHub number or URL, or a Jira
/// key.
pub fn issue(tracker: Tracker, id: &str, root: &Path) -> Result<Item, Error> {
    match tracker {
        Tracker::Github => {
            let out = run(
                "gh",
                &["issue", "view", id, "--json", GH_FIELDS],
                None,
                root,
            )?;
            let item = json::<GhItem>(&out, "gh issue view")?;
            if item.url.contains("/pull/") {
                return Err(Error::PullRequest(item.number.to_string()));
            }
            Ok(item.into())
        }
        Tracker::Jira => Jira {
            c: creds(root, "jira.local.json")?,
        }
        .issue(id, root),
    }
}

/// A pull request from the configured host. On GitHub the explicit field
/// list keeps `gh` off the deprecated Projects-Classic query.
pub fn pr(ci: Ci, n: &str, root: &Path) -> Result<Item, Error> {
    match ci {
        Ci::Github => {
            let out = run("gh", &["pr", "view", n, "--json", GH_FIELDS], None, root)?;
            Ok(json::<GhItem>(&out, "gh pr view")?.into())
        }
        Ci::Bitbucket => Bitbucket::load(root)?.pr(n, root),
    }
}

/// A pull request's head branch.
pub fn pr_branch(ci: Ci, n: &str, root: &Path) -> Result<String, Error> {
    let branch = match ci {
        Ci::Github => run(
            "gh",
            &[
                "api",
                &format!("repos/{{owner}}/{{repo}}/pulls/{n}"),
                "--jq",
                ".head.ref",
            ],
            None,
            root,
        )?
        .trim()
        .to_owned(),
        Ci::Bitbucket => {
            let bb = Bitbucket::load(root)?;
            let pr: Value = json(
                &bb.call(&[&bb.api(&format!("pullrequests/{n}"))], root)?,
                "bitbucket",
            )?;
            pr["source"]["branch"]["name"]
                .as_str()
                .unwrap_or("")
                .to_owned()
        }
    };
    if branch.is_empty() {
        return Err(Error::NotFound(format!(
            "pull request {n} has no head branch"
        )));
    }
    Ok(branch)
}

/// The number at the end of the URL `gh … create` prints.
fn number_of(url: &str) -> Result<u64, Error> {
    url.trim()
        .rsplit('/')
        .next()
        .and_then(|n| n.parse().ok())
        .ok_or_else(|| Error::Shape(format!("no number in {}", url.trim())))
}

fn github_tracker(tracker: Tracker, what: &'static str) -> Result<(), Error> {
    match tracker {
        Tracker::Github => Ok(()),
        Tracker::Jira => Err(Error::Unsupported(what)),
    }
}

fn github_ci(ci: Ci, what: &'static str) -> Result<(), Error> {
    match ci {
        Ci::Github => Ok(()),
        Ci::Bitbucket => Err(Error::Unsupported(what)),
    }
}

/// Files an issue; its number.
pub fn issue_create(tracker: Tracker, title: &str, body: &str, root: &Path) -> Result<u64, Error> {
    github_tracker(tracker, "issue create")?;
    let url = run(
        "gh",
        &["issue", "create", "--title", title, "--body-file", "-"],
        Some(body),
        root,
    )?;
    number_of(&url)
}

/// Replaces an issue's title and body.
pub fn issue_edit(
    tracker: Tracker,
    n: u64,
    title: &str,
    body: &str,
    root: &Path,
) -> Result<(), Error> {
    github_tracker(tracker, "issue edit")?;
    run(
        "gh",
        &[
            "issue",
            "edit",
            &n.to_string(),
            "--title",
            title,
            "--body-file",
            "-",
        ],
        Some(body),
        root,
    )
    .map(drop)
}

pub fn issue_comment(tracker: Tracker, n: u64, body: &str, root: &Path) -> Result<(), Error> {
    github_tracker(tracker, "issue comment")?;
    run(
        "gh",
        &["issue", "comment", &n.to_string(), "--body-file", "-"],
        Some(body),
        root,
    )
    .map(drop)
}

/// The open issue titled exactly `title`, if one exists.
pub fn issue_find_open(tracker: Tracker, title: &str, root: &Path) -> Result<Option<u64>, Error> {
    #[derive(Deserialize)]
    struct Found {
        number: u64,
        title: String,
    }
    github_tracker(tracker, "issue search")?;
    let out = run(
        "gh",
        &[
            "issue",
            "list",
            "--state",
            "open",
            "--search",
            &format!("{title} in:title"),
            "--json",
            "number,title",
        ],
        None,
        root,
    )?;
    let found: Vec<Found> = json(&out, "gh issue list")?;
    Ok(found
        .into_iter()
        .find(|f| f.title == title)
        .map(|f| f.number))
}

/// Opens a draft pull request of `branch` into `base`; its number.
pub fn pr_create_draft(
    ci: Ci,
    branch: &str,
    base: &str,
    title: &str,
    body: &str,
    root: &Path,
) -> Result<u64, Error> {
    github_ci(ci, "pr create")?;
    let url = run(
        "gh",
        &[
            "pr",
            "create",
            "--draft",
            "--head",
            branch,
            "--base",
            base,
            "--title",
            title,
            "--body-file",
            "-",
        ],
        Some(body),
        root,
    )?;
    number_of(&url)
}

/// Replaces a pull request's body, and its title when given.
pub fn pr_edit(ci: Ci, n: u64, title: Option<&str>, body: &str, root: &Path) -> Result<(), Error> {
    github_ci(ci, "pr edit")?;
    let n = n.to_string();
    let mut args = vec!["pr", "edit", n.as_str(), "--body-file", "-"];
    if let Some(t) = title {
        args.extend(["--title", t]);
    }
    run("gh", &args, Some(body), root).map(drop)
}

pub fn pr_ready(ci: Ci, n: u64, root: &Path) -> Result<(), Error> {
    github_ci(ci, "pr ready")?;
    run("gh", &["pr", "ready", &n.to_string()], None, root).map(drop)
}

/// A pull request's state (`open`, `merged`, `closed`), whether it is a
/// draft, and its body.
pub fn pr_status(ci: Ci, n: u64, root: &Path) -> Result<(String, bool, String), Error> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Pr {
        state: String,
        is_draft: bool,
        body: Option<String>,
    }
    github_ci(ci, "pr status")?;
    let out = run(
        "gh",
        &["pr", "view", &n.to_string(), "--json", "state,isDraft,body"],
        None,
        root,
    )?;
    let p: Pr = json(&out, "gh pr view")?;
    Ok((
        p.state.to_ascii_lowercase(),
        p.is_draft,
        p.body.unwrap_or_default(),
    ))
}

/// Waits for a pull request's checks; their exit code (0 green) and the
/// output, which names the failed checks.
pub fn pr_checks_watch(ci: Ci, n: u64, root: &Path) -> Result<(i32, String), Error> {
    github_ci(ci, "pr checks")?;
    let out = Command::new("gh")
        .args(["pr", "checks", &n.to_string(), "--watch"])
        .current_dir(if root.as_os_str().is_empty() {
            Path::new(".")
        } else {
            root
        })
        .stdin(Stdio::null())
        .output()
        .map_err(|e| Error::Command(format!("gh: {e}")))?;
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    Ok((out.status.code().unwrap_or(1), text))
}

/// The names of the failed checks in `gh pr checks` output.
#[must_use]
pub fn failed_checks(output: &str) -> Vec<String> {
    output
        .lines()
        .filter_map(|l| {
            let mut cells = l.split('\t');
            let name = cells.next()?.trim();
            let state = cells.next()?.trim();
            (state == "fail" && !name.is_empty()).then(|| name.to_owned())
        })
        .collect()
}

fn names_an_error(line: &str) -> bool {
    let l = line.to_ascii_lowercase();
    (l.contains("error") || l.contains("failed") || l.contains("failure"))
        && !l.contains("post job cleanup")
        && !l.contains("deprecated")
}

/// The lines of a CI log that name an error or failure, each followed by
/// the `after` lines below it; with context, `--` separates lines that are
/// not adjacent in the log.
#[must_use]
pub fn error_lines(log: &str, after: usize) -> Vec<&str> {
    let lines: Vec<&str> = log.lines().collect();
    let mut keep = vec![false; lines.len()];
    for (i, l) in lines.iter().enumerate() {
        if names_an_error(l) {
            let end = (i + after + 1).min(lines.len());
            keep[i..end].iter_mut().for_each(|k| *k = true);
        }
    }
    let mut out = Vec::new();
    let mut last = None;
    for (i, l) in lines.iter().enumerate().filter(|(i, _)| keep[*i]) {
        if after > 0 && last.is_some_and(|j: usize| j + 1 != i) {
            out.push("--");
        }
        out.push(*l);
        last = Some(i);
    }
    out
}

/// Which failed CI run and job `failed` reads.
#[derive(Debug, Clone, Copy, Default)]
pub struct Query<'a> {
    /// A run id (GitHub) or pipeline uuid or number (Bitbucket) instead of
    /// the branch's latest failure.
    pub run: Option<&'a str>,
    /// A failing job (step) whose name contains this, ignoring case,
    /// instead of the first failing one.
    pub job: Option<&'a str>,
    /// Lines printed after each error line.
    pub context: usize,
}

/// The error lines of a failed CI run: the latest on `branch` unless the
/// query names one.
pub fn failed(ci: Ci, branch: &str, query: Query, root: &Path) -> Result<Vec<String>, Error> {
    let log = match ci {
        Ci::Github => github_failed_log(branch, query, root)?,
        Ci::Bitbucket => bitbucket_failed_log(branch, query, root)?,
    };
    Ok(error_lines(&log, query.context)
        .into_iter()
        .map(str::to_owned)
        .collect())
}

/// The failing job `query` picks among `(name, id)` pairs, or why none.
fn pick_job<'a>(failing: &[(&'a str, &'a str)], query: Query, run: &str) -> Result<&'a str, Error> {
    let found = match query.job {
        Some(want) => {
            let want = want.to_lowercase();
            failing
                .iter()
                .find(|(name, _)| name.to_lowercase().contains(&want))
        }
        None => failing.first(),
    };
    found.map(|(_, id)| *id).ok_or_else(|| {
        let names: Vec<&str> = failing.iter().map(|(n, _)| *n).collect();
        Error::NotFound(match query.job {
            Some(want) => format!(
                "Run {run} has no failing job matching '{want}'; failing: {}",
                names.join(", ")
            ),
            None => format!("Run {run} failed but no failing job found"),
        })
    })
}

fn github_failed_log(branch: &str, query: Query, root: &Path) -> Result<String, Error> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Run {
        database_id: u64,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Job {
        database_id: u64,
        #[serde(default)]
        name: String,
        conclusion: Option<String>,
    }
    #[derive(Deserialize)]
    struct Jobs {
        jobs: Vec<Job>,
    }
    let id = if let Some(given) = query.run {
        given.to_owned()
    } else {
        let runs: Vec<Run> = json(
            &run(
                "gh",
                &[
                    "run",
                    "list",
                    "--branch",
                    branch,
                    "--status",
                    "failure",
                    "--limit",
                    "1",
                    "--json",
                    "databaseId",
                ],
                None,
                root,
            )?,
            "gh run list",
        )?;
        let Some(r) = runs.first() else {
            return Err(Error::NotFound(format!(
                "No failed run found for branch '{branch}'"
            )));
        };
        r.database_id.to_string()
    };
    let jobs: Jobs = json(
        &run("gh", &["run", "view", &id, "--json", "jobs"], None, root)?,
        "gh run view",
    )?;
    let ids: Vec<String> = jobs
        .jobs
        .iter()
        .map(|j| j.database_id.to_string())
        .collect();
    let failing: Vec<(&str, &str)> = jobs
        .jobs
        .iter()
        .zip(&ids)
        .filter(|(j, _)| j.conclusion.as_deref() == Some("failure"))
        .map(|(j, id)| (j.name.as_str(), id.as_str()))
        .collect();
    let job = pick_job(&failing, query, &id)?.to_owned();
    match run(
        "gh",
        &["run", "view", &id, "--job", &job, "--log-failed"],
        None,
        root,
    ) {
        Ok(log) if !log.trim().is_empty() => Ok(log),
        _ => run(
            "gh",
            &[
                "api",
                &format!("repos/{{owner}}/{{repo}}/actions/jobs/{job}/logs"),
            ],
            None,
            root,
        ),
    }
}

fn bitbucket_failed_log(branch: &str, query: Query, root: &Path) -> Result<String, Error> {
    let bb = Bitbucket::load(root)?;
    let api = bb.api("pipelines/");
    let uuid = if let Some(given) = query.run {
        given.to_owned()
    } else {
        let q = format!("target.ref_name=\"{branch}\" AND state.result.name=\"FAILED\"");
        let pipelines: Value = json(
            &bb.call(
                &[
                    "-G",
                    &api,
                    "--data-urlencode",
                    "sort=-created_on",
                    "--data-urlencode",
                    "pagelen=1",
                    "--data-urlencode",
                    &format!("q={q}"),
                ],
                root,
            )?,
            "bitbucket",
        )?;
        let Some(uuid) = pipelines["values"][0]["uuid"].as_str().map(str::to_owned) else {
            return Err(Error::NotFound(format!(
                "No failed pipeline found for branch '{branch}'"
            )));
        };
        uuid
    };
    let steps: Value = json(
        &bb.call(&["-g", &format!("{api}{uuid}/steps/")], root)?,
        "bitbucket",
    )?;
    let failing: Vec<(&str, &str)> = steps["values"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|s| s["state"]["result"]["name"] == "FAILED")
        .filter_map(|s| Some((s["name"].as_str().unwrap_or(""), s["uuid"].as_str()?)))
        .collect();
    let step = pick_job(&failing, query, &uuid)?;
    bb.call(&["-g", &format!("{api}{uuid}/steps/{step}/log")], root)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ut_github_item_as_text() {
        let g: GhItem = serde_json::from_str(
            r#"{"number":7,"title":"Retry","state":"OPEN","url":"https://x/7","author":{"login":"al"},"body":"","comments":[{"author":null,"createdAt":"2026-01-02T03:04:05Z","body":"ok"}]}"#,
        )
        .unwrap();
        let i: Item = g.into();
        assert_eq!(
            i.text(),
            "#7 Retry [OPEN]\nhttps://x/7\nby al\n\n(no body)\n\n--- 1 comment(s) ---\n\n[unknown @ 2026-01-02T03:04:05Z]\nok\n"
        );
        assert_eq!(i.title(), "Retry");
    }

    #[test]
    fn ut_adf_text_joins_leaves() {
        let v: Value = serde_json::from_str(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"a"},{"type":"text","text":"b"}]}]}"#,
        )
        .unwrap();
        assert_eq!(adf_text(&v), "a b");
    }

    #[test]
    fn ut_failed_checks_and_numbers() {
        assert_eq!(
            failed_checks("test\tfail\t1m\thttps://x\nlint\tpass\t5s\thttps://y\n"),
            ["test"]
        );
        assert_eq!(number_of("https://github.com/o/r/pull/41\n").unwrap(), 41);
        assert!(number_of("nope").is_err());
    }

    #[test]
    fn ut_error_lines() {
        assert_eq!(
            error_lines(
                "ok\nerror[E0308]: x\nPost job cleanup failed\ntest a ... FAILED\n",
                0
            ),
            ["error[E0308]: x", "test a ... FAILED"]
        );
        assert_eq!(
            error_lines("error: a\n  at 1\n  at 2\nok\nerror: b\nerror: c\nend\n", 1),
            ["error: a", "  at 1", "--", "error: b", "error: c", "end"]
        );
    }
}
