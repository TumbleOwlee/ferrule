//! The tracker and CI hosts: issues, pull requests and failed CI runs,
//! through `gh` for GitHub and `curl` for Jira and Bitbucket. Everything is
//! returned as an [`Item`] so the commands above never see a host's JSON.
//! Issues go through an [`IssueTracker`] and pull requests through a
//! [`CodeHost`], each built once from the configuration; issues and pull
//! requests are named by an [`ItemRef`]. What stays here is the plumbing
//! they share: running programs, `curl` with credentials, paging, the
//! item and error types, and reading CI logs.

mod bitbucket;
mod github;
mod host;
mod item_ref;
mod jira;
mod tracker;

pub use bitbucket::Bitbucket;
pub use github::Gh;
pub use host::{CodeHost, PrStatus, code_host};
pub use item_ref::ItemRef;
pub use jira::Jira;
pub use tracker::{FIND_LIMIT, Found, IssueTracker, issue_tracker, title_query};

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};

use serde::Deserialize;
use serde_json::Value;

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

/// Comment numbers `--comment` picks: `3`, `2-4`, `5-` or `-2`. Comments
/// are numbered from 1, oldest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommentRange {
    from: usize,
    to: Option<usize>,
}

impl CommentRange {
    fn contains(self, n: usize) -> bool {
        n >= self.from && self.to.is_none_or(|t| n <= t)
    }

    /// Whether every number it names exists among `max` comments.
    fn fits(self, max: usize) -> bool {
        self.from >= 1 && self.from <= max && self.to.is_none_or(|t| t <= max)
    }
}

impl std::str::FromStr for CommentRange {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bad = || format!("'{s}' is no comment number or range: give 3, 2-4, 5- or -2");
        let num = |t: &str| {
            if t.is_empty() || !t.bytes().all(|b| b.is_ascii_digit()) {
                return Err(bad());
            }
            t.parse::<usize>().map_err(|_| bad())
        };
        let t = s.trim();
        let range = match t.split_once('-') {
            None => {
                let n = num(t)?;
                CommentRange {
                    from: n,
                    to: Some(n),
                }
            }
            Some(("", "")) => return Err(bad()),
            Some(("", to)) => CommentRange {
                from: 1,
                to: Some(num(to)?),
            },
            Some((from, "")) => CommentRange {
                from: num(from)?,
                to: None,
            },
            Some((from, to)) => CommentRange {
                from: num(from)?,
                to: Some(num(to)?),
            },
        };
        if range.to.is_some_and(|t| t < range.from) {
            return Err(bad());
        }
        Ok(range)
    }
}

/// `--comment` named a comment the item does not have.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NoSuchComment {
    /// How many comments the item has.
    pub max: usize,
}

/// The parts of an item `issue view` and `pr view` print. Without a flag:
/// the body and an index of the comments, one line each.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Select {
    /// Only the body; with `comments` or `pick`, both.
    pub body: bool,
    /// Every comment in full.
    pub comments: bool,
    /// Only the latest n of the comments kept.
    pub last: Option<usize>,
    /// Only the comments by this author, ignoring case.
    pub author: Option<String>,
    /// Only the comments with these numbers.
    pub pick: Vec<CommentRange>,
}

impl Select {
    /// Whether comments print in full (else as an index, or not at all).
    #[must_use]
    pub fn full(&self) -> bool {
        self.asks_comments() || (!self.body && (self.last.is_some() || self.author.is_some()))
    }

    fn asks_comments(&self) -> bool {
        self.comments || !self.pick.is_empty()
    }

    #[must_use]
    pub fn shows_body(&self) -> bool {
        self.body || !self.asks_comments()
    }

    /// Whether the comment index prints: without a flag.
    #[must_use]
    pub fn shows_index(&self) -> bool {
        !self.body && !self.full()
    }

    /// The comments of `item` this selection prints in full, oldest first,
    /// each with its number among all of them.
    ///
    /// # Errors
    ///
    /// [`NoSuchComment`] when `pick` names a number the item lacks.
    pub fn comments<'a>(&self, item: &'a Item) -> Result<Vec<(usize, &'a Comment)>, NoSuchComment> {
        let max = item.comments.len();
        if self.pick.iter().any(|p| !p.fits(max)) {
            return Err(NoSuchComment { max });
        }
        let mut kept: Vec<(usize, &Comment)> = (1..)
            .zip(&item.comments)
            .filter(|(n, c)| {
                (self.pick.is_empty() || self.pick.iter().any(|p| p.contains(*n)))
                    && self
                        .author
                        .as_ref()
                        .is_none_or(|a| c.author.eq_ignore_ascii_case(a))
            })
            .collect();
        if let Some(n) = self.last {
            kept.drain(..kept.len().saturating_sub(n));
        }
        Ok(kept)
    }
}

/// `<n> comment(s)`, or `<k> of <n> comment(s)` when `k` is fewer.
fn count(k: usize, n: usize) -> String {
    if k == n {
        format!("{n} comment(s)")
    } else {
        format!("{k} of {n} comment(s)")
    }
}

/// `line` without its markdown lead: `#`, `>` and list markers.
fn unmarked(line: &str) -> &str {
    let mut l = line.trim();
    loop {
        let before = l;
        l = l.trim_start_matches(['#', '>']).trim_start();
        for marker in ["- ", "* ", "+ "] {
            l = l.strip_prefix(marker).unwrap_or(l);
        }
        let digits = l.bytes().take_while(u8::is_ascii_digit).count();
        if digits > 0
            && let Some(rest) = l[digits..]
                .strip_prefix(". ")
                .or_else(|| l[digits..].strip_prefix(") "))
        {
            l = rest;
        }
        l = l.trim_start();
        if l == before {
            return l;
        }
    }
}

impl Item {
    /// Title, body and every comment as compact plain text.
    #[must_use]
    pub fn text(&self) -> String {
        let mut out = self.head_text();
        let n = self.comments.len();
        let _ = writeln!(out, "{}\n\n--- {} ---", self.body, count(n, n));
        for c in &self.comments {
            let _ = writeln!(out, "\n[{} @ {}]\n{}", c.author, c.at, c.body);
        }
        out
    }

    /// The head, the URL when there is one and `by <author>`, then a blank
    /// line.
    fn head_text(&self) -> String {
        let mut out = format!("{}\n", self.head);
        if let Some(url) = &self.url {
            let _ = writeln!(out, "{url}");
        }
        let _ = writeln!(out, "by {}\n", self.by);
        out
    }

    /// The comment index's title: `<n> comment(s) (<view> --comment
    /// <n|a-b>)`, `<view>` being the command that prints one in full.
    #[must_use]
    pub fn index_title(&self, view: &str) -> String {
        let n = self.comments.len();
        if n == 0 {
            count(0, 0)
        } else {
            format!("{} ({view} --comment <n|a-b>)", count(n, n))
        }
    }

    /// One line per comment, at most `width` columns: its number, author
    /// and time, how many lines it has and its first line with words.
    #[must_use]
    pub fn index(&self, width: usize) -> Vec<String> {
        let w = self.comments.len().to_string().len();
        (1..)
            .zip(&self.comments)
            .map(|(n, c)| {
                let lines = c.body.lines().count();
                let unit = if lines == 1 { "line" } else { "lines" };
                let first = c
                    .body
                    .lines()
                    .map(unmarked)
                    .find(|l| !l.is_empty())
                    .unwrap_or("");
                let line = format!("{n:>w$} [{} @ {}] {lines} {unit}: {first}", c.author, c.at);
                crate::term::cut(&line, width)
            })
            .collect()
    }

    /// The title above the comments printed in full: `<k> of <n>
    /// comment(s)`, or `<n> comment(s)` when all are.
    #[must_use]
    pub fn full_title(&self, kept: usize) -> String {
        count(kept, self.comments.len())
    }

    /// The parts `select` keeps, as compact plain text; `view` and `width`
    /// as for [`Item::index_title`] and [`Item::index`]. The head, URL and
    /// author always print.
    ///
    /// # Errors
    ///
    /// [`NoSuchComment`] when `select` picks a comment the item lacks.
    pub fn text_for(
        &self,
        select: &Select,
        view: &str,
        width: usize,
    ) -> Result<String, NoSuchComment> {
        let full = if select.full() {
            Some(select.comments(self)?)
        } else {
            None
        };
        let mut out = self.head_text();
        if select.shows_body() {
            let _ = writeln!(out, "{}\n", self.body);
        }
        if select.shows_index() {
            let _ = writeln!(out, "--- {} ---", self.index_title(view));
            for l in self.index(width) {
                let _ = writeln!(out, "{l}");
            }
        }
        if let Some(kept) = full {
            let _ = writeln!(out, "--- {} ---", self.full_title(kept.len()));
            for (n, c) in kept {
                let _ = writeln!(out, "\n[#{n} {} @ {}]\n{}", c.author, c.at, c.body);
            }
        }
        Ok(out)
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

pub(crate) fn creds<T: for<'de> Deserialize<'de>>(root: &Path, file: &str) -> Result<T, Error> {
    let path = root.join(DIR).join(file);
    let text = std::fs::read_to_string(&path)
        .map_err(|e| Error::Credentials(format!("{}: {e}", path.display())))?;
    serde_json::from_str(&text).map_err(|e| Error::Credentials(format!("{}: {e}", path.display())))
}

/// `curl` with the credentials on stdin (`--config -`), never in argv. A
/// control character in them is refused: a newline would start another
/// config line.
pub(crate) fn curl(user: &str, pass: &str, args: &[&str], root: &Path) -> Result<String, Error> {
    if user.chars().chain(pass.chars()).any(char::is_control) {
        return Err(Error::Credentials(
            "the user name or token holds a control character".to_owned(),
        ));
    }
    let quote = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    let config = format!("user = \"{}:{}\"\n", quote(user), quote(pass));
    let mut all = vec!["-sf", "--config", "-"];
    all.extend_from_slice(args);
    run("curl", &all, Some(&config), root)
}

/// The most pages of a listing read before giving up: a host that never
/// stops paging must not hang a command.
const MAX_PAGES: usize = 200;

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

/// The number at the end of the URL `gh … create` prints.
pub(crate) fn number_of(url: &str) -> Result<u64, Error> {
    url.trim()
        .rsplit('/')
        .next()
        .and_then(|n| n.parse().ok())
        .ok_or_else(|| Error::Shape(format!("no number in {}", url.trim())))
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
    fn ut_comment_ranges_parse_and_fit() {
        let r = |s: &str| s.parse::<CommentRange>();
        assert_eq!(
            r("3"),
            Ok(CommentRange {
                from: 3,
                to: Some(3)
            })
        );
        assert_eq!(r("5-").unwrap().to, None);
        assert_eq!(r("-2").unwrap().from, 1);
        for bad in ["", "-", "a", "3-2", "1-b", "+1", "1--2"] {
            assert!(r(bad).is_err(), "{bad}");
        }
        assert!(r("2-4").unwrap().fits(4) && !r("2-4").unwrap().fits(3));
        assert!(!r("0").unwrap().fits(3) && !r("-1").unwrap().fits(0));
        assert!(r("3-").unwrap().contains(9) && !r("3-").unwrap().contains(2));
    }

    #[test]
    fn ut_index_aligns_numbers_and_drops_markdown_leads() {
        let comment = |body: &str| Comment {
            author: "al".to_owned(),
            at: "t".to_owned(),
            body: body.to_owned(),
        };
        let mut comments: Vec<Comment> = (0..9).map(|_| comment("ok")).collect();
        comments.push(comment("\n\n  ### > 1. **Done**\nnext"));
        let item = Item {
            head: "#1 T [OPEN]".to_owned(),
            url: None,
            open: true,
            by: "al".to_owned(),
            body: "b".to_owned(),
            comments,
        };
        let index = item.index(100);
        assert_eq!(index[0], " 1 [al @ t] 1 line: ok");
        assert_eq!(index[9], "10 [al @ t] 4 lines: **Done**");
        assert_eq!(unmarked("> - * x"), "x");
        assert_eq!(unmarked("2) y"), "y");
        assert_eq!(unmarked("2024 was"), "2024 was");
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
