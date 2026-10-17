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
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Command(e) | Error::Credentials(e) | Error::Shape(e) | Error::NotFound(e) => {
                f.write_str(e)
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

impl Item {
    /// Title, body and comments as compact plain text.
    #[must_use]
    pub fn text(&self) -> String {
        let mut out = format!("{}\n", self.head);
        if let Some(url) = &self.url {
            let _ = writeln!(out, "{url}");
        }
        let _ = writeln!(out, "by {}\n\n{}\n", self.by, self.body);
        let _ = writeln!(out, "--- {} comment(s) ---", self.comments.len());
        for c in &self.comments {
            let _ = writeln!(out, "\n[{} @ {}]\n{}", c.author, c.at, c.body);
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
            Ok(json::<GhItem>(&out, "gh issue view")?.into())
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

/// The lines of a CI log that name an error or failure.
#[must_use]
pub fn error_lines(log: &str) -> Vec<&str> {
    log.lines()
        .filter(|l| {
            let l = l.to_ascii_lowercase();
            (l.contains("error") || l.contains("failed") || l.contains("failure"))
                && !l.contains("post job cleanup")
                && !l.contains("deprecated")
        })
        .collect()
}

/// The error lines of the latest failed CI run on `branch`.
pub fn failed(ci: Ci, branch: &str, root: &Path) -> Result<Vec<String>, Error> {
    let log = match ci {
        Ci::Github => github_failed_log(branch, root)?,
        Ci::Bitbucket => bitbucket_failed_log(branch, root)?,
    };
    Ok(error_lines(&log).into_iter().map(str::to_owned).collect())
}

fn github_failed_log(branch: &str, root: &Path) -> Result<String, Error> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Run {
        database_id: u64,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Job {
        database_id: u64,
        conclusion: Option<String>,
    }
    #[derive(Deserialize)]
    struct Jobs {
        jobs: Vec<Job>,
    }
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
    let id = r.database_id.to_string();
    let jobs: Jobs = json(
        &run("gh", &["run", "view", &id, "--json", "jobs"], None, root)?,
        "gh run view",
    )?;
    let Some(job) = jobs
        .jobs
        .iter()
        .find(|j| j.conclusion.as_deref() == Some("failure"))
    else {
        return Err(Error::NotFound(format!(
            "Run {id} failed but no failing job found"
        )));
    };
    let job = job.database_id.to_string();
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

fn bitbucket_failed_log(branch: &str, root: &Path) -> Result<String, Error> {
    let bb = Bitbucket::load(root)?;
    let api = bb.api("pipelines/");
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
    let Some(uuid) = pipelines["values"][0]["uuid"].as_str() else {
        return Err(Error::NotFound(format!(
            "No failed pipeline found for branch '{branch}'"
        )));
    };
    let steps: Value = json(
        &bb.call(&["-g", &format!("{api}{uuid}/steps/")], root)?,
        "bitbucket",
    )?;
    let step = steps["values"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|s| s["state"]["result"]["name"] == "FAILED")
        .and_then(|s| s["uuid"].as_str());
    let Some(step) = step else {
        return Err(Error::NotFound(format!(
            "Pipeline {uuid} failed but no failing step found"
        )));
    };
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
    fn ut_error_lines() {
        assert_eq!(
            error_lines("ok\nerror[E0308]: x\nPost job cleanup failed\ntest a ... FAILED\n"),
            ["error[E0308]: x", "test a ... FAILED"]
        );
    }
}
