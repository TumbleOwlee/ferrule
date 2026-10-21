//! The tracker and CI hosts: issues, pull requests and failed CI runs,
//! through `gh` for GitHub and `curl` for Jira and Bitbucket. Everything is
//! returned as an [`Item`] so the commands above never see a host's JSON.
//! Pull requests go through a [`CodeHost`] built once from the
//! configuration; issues and pull requests are named by an [`ItemRef`].

mod bitbucket;
mod github;
mod host;
mod item_ref;

pub use bitbucket::Bitbucket;
pub use github::Gh;
pub use host::{CodeHost, PrStatus, code_host};
pub use item_ref::ItemRef;

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};

use serde::Deserialize;
use serde_json::Value;

use crate::config::Tracker;
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
    /// The configured host has no implementation of this operation yet.
    Unsupported {
        host: &'static str,
        op: &'static str,
    },
    /// Input that is no issue or PR reference, or a reference of the
    /// wrong kind (a Jira key for a pull request).
    Ref(String),
    /// The number names a pull request, not an issue (GitHub serves both).
    PullRequest(String),
    /// The pull request has no checks: `gh pr checks`' own words.
    NoChecks(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Command(e)
            | Error::Credentials(e)
            | Error::Shape(e)
            | Error::NotFound(e)
            | Error::NoChecks(e)
            | Error::Ref(e) => f.write_str(e),
            Error::Unsupported { op, .. } => write!(f, "{op} is only implemented for GitHub yet"),
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

pub(crate) fn json<T: for<'de> Deserialize<'de>>(text: &str, what: &str) -> Result<T, Error> {
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
    /// Not closed: `OPEN` on GitHub and Bitbucket, any status outside the
    /// `done` category on Jira.
    pub open: bool,
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
pub(crate) struct GhItem {
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

pub(crate) fn non_empty(s: Option<&str>, empty: &str) -> String {
    s.filter(|b| !b.is_empty()).unwrap_or(empty).to_owned()
}

impl From<GhItem> for Item {
    fn from(g: GhItem) -> Self {
        Item {
            head: format!("#{} {} [{}]", g.number, g.title, g.state),
            url: Some(g.url),
            open: g.state == "OPEN",
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

pub(crate) const GH_FIELDS: &str = "number,title,state,url,author,body,comments";

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

pub(crate) fn creds<T: for<'de> Deserialize<'de>>(root: &Path, file: &str) -> Result<T, Error> {
    let path = root.join(DIR).join(file);
    let text = std::fs::read_to_string(&path)
        .map_err(|e| Error::Credentials(format!("{}: {e}", path.display())))?;
    serde_json::from_str(&text).map_err(|e| Error::Credentials(format!("{}: {e}", path.display())))
}

/// `curl` with the credentials on stdin (`--config -`), never in argv.
pub(crate) fn curl(user: &str, pass: &str, args: &[&str], root: &Path) -> Result<String, Error> {
    let quote = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    let config = format!("user = \"{}:{}\"\n", quote(user), quote(pass));
    let mut all = vec!["-sf", "--config", "-"];
    all.extend_from_slice(args);
    run("curl", &all, Some(&config), root)
}

/// Whether a Jira status is not closed: its category is not `done`, whatever
/// the project names its statuses.
fn jira_open(status: &Value) -> bool {
    status["statusCategory"]["key"] != "done"
}

/// The most pages of a listing read before giving up: a host that never
/// stops paging must not hang a command.
const MAX_PAGES: usize = 200;

/// Every comment of a Jira issue: `get(start)` fetches the page from
/// `startAt=start`; pages follow until `total` is reached.
fn jira_comments(mut get: impl FnMut(usize) -> Result<Value, Error>) -> Result<Vec<Value>, Error> {
    let mut out: Vec<Value> = Vec::new();
    for _ in 0..MAX_PAGES {
        let page = get(out.len())?;
        let got = page["comments"].as_array().map_or(0, Vec::len);
        out.extend(page["comments"].as_array().into_iter().flatten().cloned());
        let total = page["total"].as_u64().unwrap_or(0);
        if got == 0 || out.len() as u64 >= total {
            return Ok(out);
        }
    }
    Err(Error::Shape(format!(
        "jira: more than {MAX_PAGES} pages of comments"
    )))
}

/// Every `values` entry of a paged Bitbucket listing, following `next`
/// from `first`; `get(url)` fetches one page.
pub(crate) fn follow_next(
    first: &str,
    mut get: impl FnMut(&str) -> Result<String, Error>,
) -> Result<Vec<Value>, Error> {
    let mut out = Vec::new();
    let mut url = first.to_owned();
    for _ in 0..MAX_PAGES {
        let page: Value = json(&get(&url)?, "bitbucket")?;
        out.extend(page["values"].as_array().into_iter().flatten().cloned());
        match page["next"].as_str() {
            Some(next) if next != url => next.clone_into(&mut url),
            _ => return Ok(out),
        }
    }
    Err(Error::Shape(format!(
        "bitbucket: more than {MAX_PAGES} pages of {first}"
    )))
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

    fn issue(&self, id: &ItemRef, root: &Path) -> Result<Item, Error> {
        let key = id.segment();
        let i = self.get(
            &format!("issue/{key}?fields=summary,status,reporter,description"),
            root,
        )?;
        let comments = jira_comments(|start| {
            self.get(
                &format!("issue/{key}/comment?orderBy=created&startAt={start}&maxResults=100"),
                root,
            )
        })?;
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
            open: jira_open(&f["status"]),
            by: f["reporter"]["displayName"]
                .as_str()
                .unwrap_or("unknown")
                .to_owned(),
            body: adf_text(&f["description"]),
            comments: comments
                .iter()
                .map(|c| Comment {
                    author: s(&c["author"]["displayName"]),
                    at: s(&c["created"]),
                    body: adf_text(&c["body"]),
                })
                .collect(),
        })
    }
}

/// An issue from the configured tracker: a GitHub number or URL, or a Jira
/// key.
pub fn issue(tracker: Tracker, id: &ItemRef, root: &Path) -> Result<Item, Error> {
    match tracker {
        Tracker::Github => {
            let out = run(
                "gh",
                &["issue", "view", &id.to_string(), "--json", GH_FIELDS],
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

/// The number at the end of the URL `gh … create` prints.
pub(crate) fn number_of(url: &str) -> Result<u64, Error> {
    url.trim()
        .rsplit('/')
        .next()
        .and_then(|n| n.parse().ok())
        .ok_or_else(|| Error::Shape(format!("no number in {}", url.trim())))
}

fn github_tracker(tracker: Tracker, what: &'static str) -> Result<(), Error> {
    match tracker {
        Tracker::Github => Ok(()),
        Tracker::Jira => Err(Error::Unsupported {
            host: "Jira",
            op: what,
        }),
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

/// The most hits [`issue_find_open`] reads.
pub const FIND_LIMIT: usize = 100;

/// The search [`issue_find_open`] sends: the title as one quoted phrase
/// (its own double quotes dropped), so words like `-v` or `is:closed` in
/// it are matched, never read as qualifiers.
#[must_use]
pub fn title_query(title: &str) -> String {
    format!("\"{}\" in:title", title.replace('"', "").trim())
}

/// The open issue titled exactly `title`, if one exists among the first
/// [`FIND_LIMIT`] hits of its quoted title.
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
            "--limit",
            &FIND_LIMIT.to_string(),
            "--search",
            &title_query(title),
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

/// One hit of [`issue_search`].
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Found {
    pub number: u64,
    pub title: String,
    pub state: String,
}

/// Issues whose title or body mention every keyword (one with a space as a
/// phrase), in `state` (`open`, `closed` or `all`); at most `limit` of them,
/// newest first.
pub fn issue_search(
    tracker: Tracker,
    keywords: &[String],
    state: &str,
    limit: usize,
    root: &Path,
) -> Result<Vec<Found>, Error> {
    github_tracker(tracker, "issue search")?;
    let words: Vec<String> = keywords
        .iter()
        .map(|k| {
            let k = k.replace('"', "");
            if k.contains(char::is_whitespace) {
                format!("\"{k}\"")
            } else {
                k
            }
        })
        .collect();
    let query = format!("{} in:title,body sort:created-desc", words.join(" "));
    let out = run(
        "gh",
        &[
            "issue",
            "list",
            "--state",
            state,
            "--search",
            &query,
            "--limit",
            &limit.to_string(),
            "--json",
            "number,title,state",
        ],
        None,
        root,
    )?;
    json(&out, "gh issue list")
}

/// The names of the failed checks in `gh pr checks` output, each once:
/// `--watch` prints a check again on every refresh.
#[must_use]
pub fn failed_checks(output: &str) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for l in output.lines() {
        let mut cells = l.split('\t');
        let (Some(name), Some(state)) = (cells.next(), cells.next()) else {
            continue;
        };
        let name = name.trim();
        if state.trim() == "fail" && !name.is_empty() && !names.iter().any(|n| n == name) {
            names.push(name.to_owned());
        }
    }
    names
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

/// The failing job `query` picks among `(name, id)` pairs, or why none.
pub(crate) fn pick_job<'a>(
    failing: &[(&'a str, &'a str)],
    query: Query,
    run: &str,
) -> Result<&'a str, Error> {
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

#[cfg(test)]
pub(crate) mod fake;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ut_failed_checks_once_each() {
        let out =
            "build\tfail\t1m\nlint\tpass\t3s\nbuild\tpending\t\nbuild\tfail\t2m\ntest\tfail\t5s\n";
        assert_eq!(failed_checks(out), ["build", "test"]);
    }

    #[test]
    fn ut_jira_comments_follow_start_at_until_total() {
        let mut starts = Vec::new();
        let got = jira_comments(|start| {
            starts.push(start);
            let all = ["a", "b", "c"];
            let page: Vec<Value> = all
                .iter()
                .skip(start)
                .take(2)
                .map(|b| serde_json::json!({ "body": b }))
                .collect();
            Ok(serde_json::json!({ "comments": page, "total": 3 }))
        })
        .unwrap();
        assert_eq!(got.len(), 3);
        assert_eq!(starts, [0, 2]);
    }

    #[test]
    fn ut_bitbucket_pages_follow_next() {
        let mut asked = Vec::new();
        let got = follow_next("u1", |u| {
            asked.push(u.to_owned());
            Ok(match u {
                "u1" => r#"{"values":[1,2],"next":"u2"}"#,
                _ => r#"{"values":[3]}"#,
            }
            .to_owned())
        })
        .unwrap();
        assert_eq!(got.len(), 3);
        assert_eq!(asked, ["u1", "u2"]);
    }

    #[test]
    fn ut_jira_closed_by_status_category() {
        let st = |c: &str| serde_json::json!({ "name": "Shipped", "statusCategory": { "key": c } });
        assert!(!jira_open(&st("done")));
        assert!(jira_open(&st("indeterminate")));
        assert!(jira_open(&st("new")));
    }

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
    fn ut_title_query_quotes_the_title() {
        assert_eq!(
            title_query("Add -v and is:closed \"modes\""),
            "\"Add -v and is:closed modes\" in:title"
        );
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
