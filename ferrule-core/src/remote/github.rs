//! GitHub as the code host and the issue tracker, through `gh`. The explicit `--json` field
//! lists keep `gh` off the deprecated Projects-Classic query.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use super::host::{Checks, CodeHost, PrState, PrStatus, comment_id};
use super::milestones::{Blocker, IssueLinks, Milestone, Milestones, NewIssue};
use super::tracker::{FIND_LIMIT, Found, IssueTracker, SearchState, title_query};
use super::{
    Error, GH_FIELDS, GhItem, Item, ItemRef, MAX_PAGES, Query, json, number_of, pick_job, run,
    run_bounded,
};
use super::{Fetched, Thread};

/// `gh` in the repository's checkout.
pub struct Gh {
    root: PathBuf,
    /// How long `checks_watch` waits.
    wait: std::time::Duration,
}

impl Gh {
    #[must_use]
    pub fn new(root: &Path) -> Self {
        Gh {
            root: root.to_owned(),
            wait: std::time::Duration::from_hours(2),
        }
    }

    /// `checks_watch` gives up after `wait`.
    #[must_use]
    pub fn waiting(mut self, wait: std::time::Duration) -> Self {
        self.wait = wait;
        self
    }

    fn gh(&self, args: &[&str], stdin: Option<&str>) -> Result<String, Error> {
        run("gh", args, stdin, &self.root)
    }
}

impl CodeHost for Gh {
    fn name(&self) -> &'static str {
        "GitHub"
    }

    fn view(&self, pr: &ItemRef) -> Result<Item, Error> {
        pr.pr_number()?;
        let out = self.gh(&["pr", "view", &pr.to_string(), "--json", GH_FIELDS], None)?;
        Ok(json::<GhItem>(&out, "gh pr view")?.into())
    }

    fn branch(&self, pr: &ItemRef) -> Result<String, Error> {
        let n = pr.pr_number()?;
        let repo = repo_of(pr)?;
        let branch = self
            .gh(
                &[
                    "api",
                    &format!("repos/{repo}/pulls/{n}"),
                    "--jq",
                    ".head.ref",
                ],
                None,
            )?
            .trim()
            .to_owned();
        if branch.is_empty() {
            return Err(Error::NotFound(format!(
                "pull request {pr} has no head branch"
            )));
        }
        Ok(branch)
    }

    /// The open PR from `branch` of this repository: `--head` matches the
    /// branch name only, so a fork's PR with the same name is skipped.
    fn find_open(&self, branch: &str) -> Result<Option<u64>, Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Found {
            number: u64,
            #[serde(default)]
            is_cross_repository: bool,
        }
        let out = self.gh(
            &[
                "pr",
                "list",
                "--head",
                branch,
                "--state",
                "open",
                "--json",
                "number,isCrossRepository",
            ],
            None,
        )?;
        let found: Vec<Found> = json(&out, "gh pr list")?;
        Ok(found
            .iter()
            .find(|f| !f.is_cross_repository)
            .map(|f| f.number))
    }

    fn create_draft(
        &self,
        branch: &str,
        base: &str,
        title: &str,
        body: &str,
    ) -> Result<u64, Error> {
        let url = self.gh(
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
        )?;
        number_of(&url)
    }

    fn edit(&self, pr: &ItemRef, title: Option<&str>, body: &str) -> Result<(), Error> {
        pr.pr_number()?;
        let n = pr.to_string();
        let mut args = vec!["pr", "edit", n.as_str(), "--body-file", "-"];
        if let Some(t) = title {
            args.extend(["--title", t]);
        }
        self.gh(&args, Some(body)).map(drop)
    }

    fn ready(&self, pr: &ItemRef) -> Result<(), Error> {
        pr.pr_number()?;
        let n = pr.to_string();
        self.gh(&["pr", "ready", &n], None).map(drop)
    }

    fn close(&self, pr: &ItemRef, comment: &str) -> Result<(), Error> {
        pr.pr_number()?;
        self.gh(
            &["pr", "close", &pr.to_string(), "--comment", comment],
            None,
        )
        .map(drop)
    }

    fn status(&self, pr: &ItemRef) -> Result<PrStatus, Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Pr {
            state: String,
            is_draft: bool,
            body: Option<String>,
        }
        pr.pr_number()?;
        let n = pr.to_string();
        let out = self.gh(&["pr", "view", &n, "--json", "state,isDraft,body"], None)?;
        let p: Pr = json(&out, "gh pr view")?;
        Ok(PrStatus {
            state: PrState::parse(&p.state),
            draft: p.is_draft,
            body: p.body.unwrap_or_default(),
        })
    }

    /// The checks' exit code and table. gh fails the same way for a red
    /// check and for a call that never read any (auth, a missing PR): only
    /// output holding check rows is a result.
    fn checks_watch(&self, pr: &ItemRef) -> Result<Checks, Error> {
        pr.pr_number()?;
        let n = pr.to_string();
        let (code, stdout, stderr) = run_bounded(
            "gh",
            &["pr", "checks", &n, "--watch"],
            &self.root,
            self.wait,
        )?;
        let text = format!("{stdout}{stderr}");
        if code != 0 && no_checks(&text) {
            return Err(Error::NoChecks(text.trim().to_owned()));
        }
        if code != 0 && !stdout.lines().any(|l| l.contains('\t')) {
            let why = stderr.trim();
            return Err(Error::Command(if why.is_empty() {
                format!("gh pr checks exited {code} without a result")
            } else {
                why.to_owned()
            }));
        }
        Ok(Checks {
            code,
            failed: super::failed_checks(&text),
            raw: text,
        })
    }

    fn threads(&self, pr: &ItemRef) -> Result<Fetched, Error> {
        let n = pr.pr_number()?;
        let repo = repo_of(pr)?;
        let (state, draft, all) = threads(
            |after| self.threads_page(&repo, n, after),
            |id, after| self.comments_page(id, after),
        )?;
        Ok(Fetched {
            state: state.to_ascii_lowercase(),
            draft,
            threads: all
                .into_iter()
                .filter(|t| !t.is_resolved)
                .map(GqlThread::into_thread)
                .collect(),
        })
    }

    fn reply(&self, pr: &ItemRef, comment: &str, text: &str) -> Result<(), Error> {
        let n = pr.pr_number()?;
        let c = comment_id(comment)?;
        let repo = repo_of(pr)?;
        let path = format!("repos/{repo}/pulls/{n}/comments/{c}/replies");
        self.gh(&["api", &path, "-f", &format!("body={text}")], None)
            .map(drop)
    }

    fn resolve(&self, pr: &ItemRef, thread: &str, _comment: &str) -> Result<(), Error> {
        pr.pr_number()?;
        repo_of(pr)?;
        // `-f`, never `-F`: `-F` reads a value starting with `@` as a file.
        self.gh(
            &[
                "api",
                "graphql",
                "-f",
                &format!("id={}", thread_id(thread)?),
                "-f",
                &format!("query={RESOLVE}"),
            ],
            None,
        )
        .map(drop)
    }

    fn failed_log(&self, branch: &str, query: Query) -> Result<String, Error> {
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
            run_id(given)?.to_owned()
        } else {
            // The branch's head: an older commit's failure is not this one's.
            let sha = self
                .gh(
                    &[
                        "api",
                        &format!("repos/{{owner}}/{{repo}}/commits/{}", ref_path(branch)?),
                        "--jq",
                        ".sha",
                    ],
                    None,
                )?
                .trim()
                .to_owned();
            let runs: Vec<Run> = json(
                &self.gh(
                    &[
                        "run",
                        "list",
                        &format!("--branch={branch}"),
                        "--commit",
                        &sha,
                        "--status",
                        "failure",
                        "--limit",
                        "1",
                        "--json",
                        "databaseId",
                    ],
                    None,
                )?,
                "gh run list",
            )?;
            let Some(r) = runs.first() else {
                return Err(Error::NotFound(format!(
                    "No failed run found for branch '{branch}' at {}",
                    sha.get(..7).unwrap_or(&sha)
                )));
            };
            r.database_id.to_string()
        };
        let jobs: Jobs = json(
            &self.gh(&["run", "view", &id, "--json", "jobs"], None)?,
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
            .filter(|(j, _)| {
                matches!(
                    j.conclusion.as_deref(),
                    Some("failure" | "timed_out" | "cancelled" | "startup_failure")
                )
            })
            .map(|(j, id)| (j.name.as_str(), id.as_str()))
            .collect();
        let job = pick_job(&failing, query, &id)?.to_owned();
        match self.gh(&["run", "view", &id, "--job", &job, "--log-failed"], None) {
            Ok(log) if !log.trim().is_empty() => Ok(log),
            _ => self.gh(
                &[
                    "api",
                    &format!("repos/{{owner}}/{{repo}}/actions/jobs/{job}/logs"),
                ],
                None,
            ),
        }
    }
}

impl IssueTracker for Gh {
    fn name(&self) -> &'static str {
        "GitHub"
    }

    fn view(&self, issue: &ItemRef) -> Result<Item, Error> {
        let out = self.gh(
            &["issue", "view", &issue.to_string(), "--json", GH_FIELDS],
            None,
        )?;
        let item = json::<GhItem>(&out, "gh issue view")?;
        if item.url.contains("/pull/") {
            return Err(Error::PullRequest(item.number.to_string()));
        }
        Ok(item.into())
    }

    fn create(&self, title: &str, body: &str) -> Result<ItemRef, Error> {
        let url = self.gh(
            &["issue", "create", "--title", title, "--body-file", "-"],
            Some(body),
        )?;
        number_of(&url).map(ItemRef::Number)
    }

    fn edit(&self, issue: &ItemRef, title: &str, body: &str) -> Result<(), Error> {
        self.gh(
            &[
                "issue",
                "edit",
                &issue.to_string(),
                "--title",
                title,
                "--body-file",
                "-",
            ],
            Some(body),
        )
        .map(drop)
    }

    fn comment(&self, issue: &ItemRef, body: &str) -> Result<(), Error> {
        self.gh(
            &["issue", "comment", &issue.to_string(), "--body-file", "-"],
            Some(body),
        )
        .map(drop)
    }

    fn find_open(&self, title: &str) -> Result<Option<ItemRef>, Error> {
        #[derive(Deserialize)]
        struct Hit {
            number: u64,
            title: String,
        }
        // Only the user's own issues: another's with the same title is not
        // the one a crash left.
        let out = self.gh(
            &[
                "issue",
                "list",
                "--state",
                "open",
                "--author",
                "@me",
                "--limit",
                &FIND_LIMIT.to_string(),
                "--search",
                &title_query(title),
                "--json",
                "number,title",
            ],
            None,
        )?;
        let found: Vec<Hit> = json(&out, "gh issue list")?;
        Ok(found
            .into_iter()
            .find(|f| f.title == title)
            .map(|f| ItemRef::Number(f.number)))
    }

    fn search(
        &self,
        keywords: &[String],
        state: SearchState,
        limit: usize,
    ) -> Result<Vec<Found>, Error> {
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
        let out = self.gh(
            &[
                "issue",
                "list",
                "--state",
                state.as_str(),
                "--search",
                &query,
                "--limit",
                &limit.to_string(),
                "--json",
                "number,title,state",
            ],
            None,
        )?;
        json(&out, "gh issue list")
    }

    fn milestones(&self) -> Option<&dyn Milestones> {
        Some(self)
    }
}

/// Whether `gh pr checks` output says the pull request has no checks (it
/// exits 1 then, as for a red check).
fn no_checks(output: &str) -> bool {
    output.contains("no checks reported")
}

/// The repository a reference names, as the `repos/<owner>/<repo>` an API
/// path starts with: a URL's own (`https://github.com/o/r/pull/7`), the
/// checkout's (`{owner}/{repo}`, which gh fills in) for a number. A URL of
/// another shape is refused, so a number never lands on another repo's
/// item of the same number.
fn repo_of(item: &ItemRef) -> Result<String, Error> {
    let ItemRef::Url { url, .. } = item else {
        return Ok("{owner}/{repo}".to_owned());
    };
    let path = url
        .split_once("://")
        .map_or(url.as_str(), |(_, rest)| rest)
        .split(['?', '#'])
        .next()
        .unwrap_or_default();
    let segments: Vec<&str> = path.trim_end_matches('/').split('/').collect();
    let name = |s: &str| {
        !s.is_empty()
            && !s.starts_with('.')
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    };
    match segments.as_slice() {
        [_host, owner, repo, "pull" | "pulls" | "issues", _n] if name(owner) && name(repo) => {
            Ok(format!("{owner}/{repo}"))
        }
        _ => Err(Error::Ref(format!(
            "{url} is not a GitHub pull request or issue URL"
        ))),
    }
}

/// A review thread's GraphQL node id (`PRRT_kwDO…`): only the characters
/// such an id holds, so a value from an edited file cannot pass as
/// anything else.
fn thread_id(thread: &str) -> Result<&str, Error> {
    if !thread.is_empty()
        && thread
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'='))
    {
        Ok(thread)
    } else {
        Err(Error::Ref(format!("'{thread}' is not a review thread id")))
    }
}

/// A workflow run id, as `ci failed --run` names one: digits only.
fn run_id(given: &str) -> Result<&str, Error> {
    if !given.is_empty() && given.bytes().all(|b| b.is_ascii_digit()) {
        Ok(given)
    } else {
        Err(Error::Ref(format!("'{given}' is not a workflow run id")))
    }
}

/// A branch as an API path segment: the characters a branch name takes in
/// practice, never a leading `-`, `..`, `?`, `#` or `%`, so it names one
/// ref and nothing else.
fn ref_path(branch: &str) -> Result<&str, Error> {
    if !branch.is_empty()
        && !branch.starts_with('-')
        && !branch.contains("..")
        && branch.bytes().all(|b| {
            b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'/' | b'+' | b'@')
        })
    {
        Ok(branch)
    } else {
        Err(Error::Ref(format!("'{branch}' is not a branch name")))
    }
}

#[derive(Deserialize)]
struct GqlRoot {
    data: GqlData,
}

#[derive(Deserialize)]
struct GqlData {
    repository: GqlRepo,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GqlRepo {
    pull_request: GqlPr,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GqlPr {
    state: String,
    is_draft: bool,
    review_threads: Nodes<GqlThread>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Nodes<T> {
    nodes: Vec<T>,
    #[serde(default)]
    page_info: PageInfo,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct PageInfo {
    has_next_page: bool,
    end_cursor: Option<String>,
}

impl PageInfo {
    /// The cursor of the next page, if there is one.
    fn next(&self) -> Result<Option<&str>, Error> {
        match (self.has_next_page, self.end_cursor.as_deref()) {
            (false, _) => Ok(None),
            (true, Some(c)) => Ok(Some(c)),
            (true, None) => Err(Error::Shape(
                "gh api graphql: no cursor for next page".into(),
            )),
        }
    }
}

#[derive(Deserialize)]
struct GqlNodeRoot {
    data: GqlNode,
}

#[derive(Deserialize)]
struct GqlNode {
    node: GqlThreadComments,
}

#[derive(Deserialize)]
struct GqlThreadComments {
    comments: Nodes<GqlComment>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GqlThread {
    id: String,
    is_resolved: bool,
    path: String,
    line: Option<u64>,
    original_line: Option<u64>,
    comments: Nodes<GqlComment>,
}

impl GqlThread {
    fn into_thread(self) -> Thread {
        Thread {
            comment: self
                .comments
                .nodes
                .first()
                .map_or_else(String::new, |c| c.database_id.to_string()),
            path: format!(
                "{}:{}",
                self.path,
                self.line
                    .or(self.original_line)
                    .map_or_else(|| "?".to_owned(), |l| l.to_string())
            ),
            comments: self
                .comments
                .nodes
                .iter()
                .map(|c| {
                    (
                        c.author
                            .as_ref()
                            .and_then(|a| a.login.clone())
                            .unwrap_or_else(|| "unknown".to_owned()),
                        c.created_at.clone(),
                        c.body
                            .clone()
                            .filter(|b| !b.is_empty())
                            .unwrap_or_else(|| "(empty)".to_owned()),
                    )
                })
                .collect(),
            id: self.id,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GqlComment {
    database_id: u64,
    author: Option<GqlAuthor>,
    created_at: String,
    body: Option<String>,
}

#[derive(Deserialize)]
struct GqlAuthor {
    login: Option<String>,
}

const THREADS_QUERY: &str = "query($owner: String!, $repo: String!, $pr: Int!, $after: String) {
  repository(owner: $owner, name: $repo) {
    pullRequest(number: $pr) {
      state isDraft
      reviewThreads(first: 100, after: $after) {
        pageInfo { hasNextPage endCursor }
        nodes {
          id isResolved path line originalLine
          comments(first: 50) {
            pageInfo { hasNextPage endCursor }
            nodes { databaseId author { login } createdAt body }
          }
        }
      }
    }
  }
}";

/// The rest of one thread's comments, from `$after`.
const COMMENTS_QUERY: &str = "query($id: ID!, $after: String) {
  node(id: $id) {
    ... on PullRequestReviewThread {
      comments(first: 50, after: $after) {
        pageInfo { hasNextPage endCursor }
        nodes { databaseId author { login } createdAt body }
      }
    }
  }
}";

const RESOLVE: &str =
    "mutation($id: ID!) { resolveReviewThread(input: {threadId: $id}) { thread { isResolved } } }";

impl Gh {
    /// The gh call for one page: `after` is the cursor of the page before.
    fn threads_page(&self, repo: &str, pr: u64, after: Option<&str>) -> Result<String, Error> {
        let (pr, query) = (format!("pr={pr}"), format!("query={THREADS_QUERY}"));
        let (owner, name) = repo.split_once('/').unwrap_or((repo, ""));
        let (owner, name) = (format!("owner={owner}"), format!("repo={name}"));
        let mut args = vec!["api", "graphql", "-F", &owner, "-F", &name, "-F", &pr];
        let cursor = after.map(|c| format!("after={c}"));
        if let Some(c) = &cursor {
            args.extend(["-f", c]);
        }
        args.extend(["-f", &query]);
        self.gh(&args, None)
    }

    /// The comments of thread `id` after the cursor `after`.
    fn comments_page(&self, id: &str, after: &str) -> Result<String, Error> {
        self.gh(
            &[
                "api",
                "graphql",
                "-f",
                &format!("id={id}"),
                "-f",
                &format!("after={after}"),
                "-f",
                &format!("query={COMMENTS_QUERY}"),
            ],
            None,
        )
    }
}

/// Every review thread of the PR and, in each, every comment: pages are
/// followed while `pageInfo` says more. `threads(after)` and
/// `comments(thread, after)` fetch one page each.
fn threads(
    mut threads: impl FnMut(Option<&str>) -> Result<String, Error>,
    mut comments: impl FnMut(&str, &str) -> Result<String, Error>,
) -> Result<(String, bool, Vec<GqlThread>), Error> {
    let shape = |e: serde_json::Error| Error::Shape(format!("gh api graphql: {e}"));
    let mut cursor: Option<String> = None;
    let mut all: Vec<GqlThread> = Vec::new();
    let mut head = None;
    for _ in 0..MAX_PAGES {
        let g: GqlRoot = serde_json::from_str(&threads(cursor.as_deref())?).map_err(shape)?;
        let p = g.data.repository.pull_request;
        head.get_or_insert((p.state, p.is_draft));
        let next = p.review_threads.page_info.next()?.map(str::to_owned);
        all.extend(p.review_threads.nodes);
        if next.is_none() {
            let (state, draft) = head.unwrap_or_default();
            for t in &mut all {
                let mut more = t.comments.page_info.next()?.map(str::to_owned);
                for _ in 0..MAX_PAGES {
                    let Some(after) = more else { break };
                    let n: GqlNodeRoot =
                        serde_json::from_str(&comments(&t.id, &after)?).map_err(shape)?;
                    let page = n.data.node.comments;
                    more = page.page_info.next()?.map(str::to_owned);
                    t.comments.nodes.extend(page.nodes);
                }
                if more.is_some() {
                    return Err(Error::Shape(format!(
                        "gh api graphql: more than {MAX_PAGES} pages of comments on {}",
                        t.id
                    )));
                }
            }
            return Ok((state, draft, all));
        }
        cursor = next;
    }
    Err(Error::Shape(format!(
        "gh api graphql: more than {MAX_PAGES} pages of review threads"
    )))
}

/// The labels of `want` that `have` lacks, compared without case as GitHub
/// does.
fn missing_labels<'a>(have: &[String], want: &'a [String]) -> Vec<&'a str> {
    want.iter()
        .filter(|l| !have.iter().any(|h| h.eq_ignore_ascii_case(l)))
        .map(String::as_str)
        .collect()
}

/// `key=value`, built apart from a literal so `gh` gets one argument.
fn kv(key: &str, value: impl std::fmt::Display) -> String {
    format!("{key}={value}")
}

/// The call creating label `name` with `color`.
fn new_label_args(name: &str, color: &str) -> Vec<String> {
    [
        "api",
        "-X",
        "POST",
        "repos/{owner}/{repo}/labels",
        "-f",
        &kv("name", name),
        "-f",
        &kv("color", color),
    ]
    .map(str::to_owned)
    .to_vec()
}

/// Six random hex digits, a label's color.
fn random_color() -> String {
    use std::hash::{BuildHasher, Hasher};
    let bits = std::collections::hash_map::RandomState::new()
        .build_hasher()
        .finish();
    format!("{:06x}", bits & 0x00ff_ffff)
}

/// One milestone as the REST API answers it.
#[derive(Deserialize)]
struct RestMilestone {
    number: u64,
    title: String,
    description: Option<String>,
    state: String,
}

impl From<RestMilestone> for Milestone {
    fn from(m: RestMilestone) -> Self {
        Milestone {
            number: m.number,
            title: m.title,
            description: m.description.unwrap_or_default(),
            open: m.state.eq_ignore_ascii_case("open"),
        }
    }
}

/// The milestones of a `--paginate --slurp` answer: an array of pages.
fn milestones_of(text: &str) -> Result<Vec<Milestone>, Error> {
    let pages: Vec<Vec<RestMilestone>> = json(text, "gh api milestones")?;
    Ok(pages.into_iter().flatten().map(Milestone::from).collect())
}

/// The issues, pull requests dropped, of a `--paginate --slurp` answer:
/// number and title, ascending by number.
fn issues_of(text: &str) -> Result<Vec<(u64, String)>, Error> {
    #[derive(Deserialize)]
    struct Issue {
        number: u64,
        title: String,
        pull_request: Option<serde_json::Value>,
    }
    let pages: Vec<Vec<Issue>> = json(text, "gh api issues")?;
    let mut all: Vec<(u64, String)> = pages
        .into_iter()
        .flatten()
        .filter(|i| i.pull_request.is_none())
        .map(|i| (i.number, i.title))
        .collect();
    all.sort_unstable_by_key(|(n, _)| *n);
    Ok(all)
}

/// A write of a milestone description GitHub refused with a 422 is
/// [`Error::Rejected`], with the text's length in characters; any other
/// error passes unchanged.
fn rejected(e: Error, description: &str) -> Error {
    match e {
        Error::Command(detail) if detail.contains("HTTP 422") => Error::Rejected {
            length: description.chars().count(),
            detail,
        },
        other => other,
    }
}

#[derive(Deserialize)]
struct LinkRepo {
    #[serde(rename = "nameWithOwner")]
    name_with_owner: String,
}

#[derive(Deserialize)]
struct LinkRef {
    number: u64,
    url: String,
    repository: LinkRepo,
    #[serde(default)]
    state: String,
}

#[derive(Deserialize)]
struct LinkNodes<T> {
    nodes: Vec<T>,
}

#[derive(Deserialize)]
struct LinkSummary {
    #[serde(rename = "blockedBy")]
    blocked_by: u32,
}

#[derive(Deserialize)]
struct LinkName {
    name: String,
}

#[derive(Deserialize)]
struct LinkIssue {
    state: String,
    milestone: Option<RestMilestone>,
    labels: LinkNodes<LinkName>,
    parent: Option<LinkRef>,
    #[serde(rename = "blockedBy")]
    blocked_by: LinkNodes<LinkRef>,
    #[serde(rename = "issueDependenciesSummary")]
    summary: LinkSummary,
    #[serde(rename = "relatesTo")]
    relates_to: LinkNodes<LinkRef>,
}

#[derive(Deserialize)]
struct LinkLinksRepo {
    #[serde(rename = "nameWithOwner")]
    name_with_owner: String,
    issue: LinkIssue,
}

/// An issue's links from the `LINKS` answer. An item of the queried
/// repository is a number citing `#7`; one of another repository is a URL
/// citing `owner/repo#7`.
fn links_of(text: &str) -> Result<IssueLinks, Error> {
    #[derive(Deserialize)]
    struct Data {
        repository: LinkLinksRepo,
    }
    #[derive(Deserialize)]
    struct Root {
        data: Data,
    }
    let repo: LinkLinksRepo = json::<Root>(text, "gh api graphql")?.data.repository;
    let here = repo.name_with_owner;
    let cite = |r: &LinkRef| {
        if r.repository.name_with_owner == here {
            (format!("#{}", r.number), ItemRef::Number(r.number))
        } else {
            (
                format!("{}#{}", r.repository.name_with_owner, r.number),
                ItemRef::Url {
                    url: r.url.clone(),
                    number: r.number,
                },
            )
        }
    };
    let issue = repo.issue;
    let blocked_by: Vec<Blocker> = issue
        .blocked_by
        .nodes
        .iter()
        .map(|b| {
            let (cite, reference) = cite(b);
            Blocker {
                cite,
                reference,
                open: b.state.eq_ignore_ascii_case("open"),
            }
        })
        .collect();
    let shown = u32::try_from(blocked_by.iter().filter(|b| b.open).count()).unwrap_or(u32::MAX);
    Ok(IssueLinks {
        open: issue.state.eq_ignore_ascii_case("open"),
        milestone: issue.milestone.map(Milestone::from),
        labels: issue.labels.nodes.into_iter().map(|l| l.name).collect(),
        parent: issue.parent.as_ref().map(|p| cite(p).1),
        unreadable: issue.summary.blocked_by.saturating_sub(shown),
        blocked_by,
        relates_to: issue.relates_to.nodes.iter().map(|r| cite(r).1).collect(),
    })
}

const LINKS: &str = "query($owner: String!, $repo: String!, $number: Int!) {
  repository(owner: $owner, name: $repo) {
    nameWithOwner
    issue(number: $number) {
      state
      milestone { number title description state }
      labels(first: 100) { nodes { name } }
      parent { number url repository { nameWithOwner } }
      blockedBy(first: 100) { nodes { number url state repository { nameWithOwner } } }
      issueDependenciesSummary { blockedBy }
      relatesTo(first: 100) { nodes { number url repository { nameWithOwner } } }
    }
  }
}";

const ISSUE_ID: &str = "query($owner: String!, $repo: String!, $number: Int!) {
  repository(owner: $owner, name: $repo) { issue(number: $number) { id } }
}";

const ADD_BLOCKED_BY: &str = "mutation($issue: ID!, $other: ID!) { addBlockedBy(input: {issueId: $issue, blockingIssueId: $other}) { clientMutationId } }";
const ADD_SUB_ISSUE: &str = "mutation($issue: ID!, $other: ID!) { addSubIssue(input: {issueId: $other, subIssueId: $issue}) { clientMutationId } }";
const ADD_RELATES_TO: &str = "mutation($issue: ID!, $other: ID!) { addRelatesTo(input: {issueId: $issue, relatedIssueId: $other}) { clientMutationId } }";

impl Gh {
    fn milestone_json(&self, args: &[&str], stdin: Option<&str>) -> Result<Milestone, Error> {
        let out = self.gh(args, stdin)?;
        Ok(json::<RestMilestone>(&out, "gh api milestone")?.into())
    }

    /// A GraphQL node id of an issue of the checkout's repository, or of
    /// the one its URL names.
    fn node_id(&self, issue: &ItemRef) -> Result<String, Error> {
        #[derive(Deserialize)]
        struct Id {
            id: String,
        }
        #[derive(Deserialize)]
        struct Repo {
            issue: Id,
        }
        #[derive(Deserialize)]
        struct Data {
            repository: Repo,
        }
        #[derive(Deserialize)]
        struct Root {
            data: Data,
        }
        let (owner, name, number) = Self::coordinates(issue)?;
        let out = self.gh(
            &[
                "api",
                "graphql",
                "-F",
                &kv("owner", owner),
                "-F",
                &kv("repo", name),
                "-F",
                &kv("number", number),
                "-f",
                &kv("query", ISSUE_ID),
            ],
            None,
        )?;
        Ok(json::<Root>(&out, "gh api graphql")?
            .data
            .repository
            .issue
            .id)
    }

    /// Owner, repository and number of an issue reference.
    fn coordinates(issue: &ItemRef) -> Result<(String, String, u64), Error> {
        let number = issue
            .number()
            .ok_or_else(|| Error::Ref(format!("{issue} is not a GitHub issue")))?;
        let repo = repo_of(issue)?;
        let (owner, name) = repo.split_once('/').unwrap_or((&repo, ""));
        Ok((owner.to_owned(), name.to_owned(), number))
    }

    /// One relationship mutation between two issues.
    fn link(&self, mutation: &str, issue: &ItemRef, other: &ItemRef) -> Result<(), Error> {
        let (a, b) = (self.node_id(issue)?, self.node_id(other)?);
        self.gh(
            &[
                "api",
                "graphql",
                "-f",
                &kv("issue", a),
                "-f",
                &kv("other", b),
                "-f",
                &kv("query", mutation),
            ],
            None,
        )
        .map(drop)
    }
}

impl Milestones for Gh {
    fn login(&self) -> Result<String, Error> {
        Ok(self
            .gh(&["api", "user", "--jq", ".login"], None)?
            .trim()
            .to_owned())
    }

    fn list(&self, state: SearchState) -> Result<Vec<Milestone>, Error> {
        let path = format!(
            "repos/{{owner}}/{{repo}}/milestones?state={}&per_page=100",
            state.as_str()
        );
        milestones_of(&self.gh(&["api", "--paginate", "--slurp", &path], None)?)
    }

    fn get(&self, number: u64) -> Result<Milestone, Error> {
        let path = format!("repos/{{owner}}/{{repo}}/milestones/{number}");
        self.milestone_json(&["api", &path], None)
    }

    fn create(&self, title: &str, description: &str) -> Result<Milestone, Error> {
        let body = serde_json::json!({"title": title, "description": description}).to_string();
        self.milestone_json(
            &[
                "api",
                "-X",
                "POST",
                "repos/{owner}/{repo}/milestones",
                "--input",
                "-",
            ],
            Some(&body),
        )
        .map_err(|e| rejected(e, description))
    }

    fn set_description(&self, number: u64, description: &str) -> Result<(), Error> {
        let path = format!("repos/{{owner}}/{{repo}}/milestones/{number}");
        let body = serde_json::json!({"description": description}).to_string();
        self.gh(&["api", "-X", "PATCH", &path, "--input", "-"], Some(&body))
            .map(drop)
            .map_err(|e| rejected(e, description))
    }

    fn issues_in(&self, number: u64) -> Result<Vec<u64>, Error> {
        let path =
            format!("repos/{{owner}}/{{repo}}/issues?milestone={number}&state=all&per_page=100");
        let all = issues_of(&self.gh(&["api", "--paginate", "--slurp", &path], None)?)?;
        Ok(all.into_iter().map(|(n, _)| n).collect())
    }

    fn find_open_in(&self, milestone: u64, title: &str) -> Result<Option<ItemRef>, Error> {
        let path = format!(
            "repos/{{owner}}/{{repo}}/issues?milestone={milestone}&state=open&per_page=100"
        );
        let all = issues_of(&self.gh(&["api", "--paginate", "--slurp", &path], None)?)?;
        Ok(all
            .into_iter()
            .find(|(_, t)| t == title)
            .map(|(n, _)| ItemRef::Number(n)))
    }

    fn links(&self, issue: &ItemRef) -> Result<IssueLinks, Error> {
        let (owner, name, number) = Self::coordinates(issue)?;
        let out = self.gh(
            &[
                "api",
                "graphql",
                "-F",
                &kv("owner", owner),
                "-F",
                &kv("repo", name),
                "-F",
                &kv("number", number),
                "-f",
                &kv("query", LINKS),
            ],
            None,
        )?;
        links_of(&out)
    }

    fn create_issue(&self, new: &NewIssue<'_>) -> Result<ItemRef, Error> {
        let mut args = vec!["issue", "create", "--title", new.title, "--body-file", "-"];
        if let Some(m) = new.milestone {
            args.extend(["--milestone", m]);
        }
        for l in new.labels {
            args.extend(["--label", l]);
        }
        let url = self.gh(&args, Some(new.body))?;
        number_of(&url).map(ItemRef::Number)
    }

    fn set_milestone(&self, issue: &ItemRef, title: &str) -> Result<(), Error> {
        self.gh(
            &["issue", "edit", &issue.to_string(), "--milestone", title],
            None,
        )
        .map(drop)
    }

    fn ensure_labels(&self, labels: &[String]) -> Result<(), Error> {
        #[derive(Deserialize)]
        struct Label {
            name: String,
        }
        if labels.is_empty() {
            return Ok(());
        }
        let out = self.gh(
            &[
                "api",
                "--paginate",
                "--slurp",
                "repos/{owner}/{repo}/labels?per_page=100",
            ],
            None,
        )?;
        let pages: Vec<Vec<Label>> = json(&out, "gh api labels")?;
        let have: Vec<String> = pages.into_iter().flatten().map(|l| l.name).collect();
        for l in missing_labels(&have, labels) {
            let args = new_label_args(l, &random_color());
            let args: Vec<&str> = args.iter().map(String::as_str).collect();
            self.gh(&args, None)?;
        }
        Ok(())
    }

    fn add_labels(&self, issue: &ItemRef, labels: &[String]) -> Result<(), Error> {
        if labels.is_empty() {
            return Ok(());
        }
        let (owner, name, number) = Self::coordinates(issue)?;
        let path = format!("repos/{owner}/{name}/issues/{number}/labels");
        let body = serde_json::json!({ "labels": labels }).to_string();
        self.gh(&["api", "-X", "POST", &path, "--input", "-"], Some(&body))
            .map(drop)
    }

    fn add_blocked_by(&self, issue: &ItemRef, blocker: &ItemRef) -> Result<(), Error> {
        self.link(ADD_BLOCKED_BY, issue, blocker)
    }

    fn set_parent(&self, issue: &ItemRef, parent: &ItemRef) -> Result<(), Error> {
        self.link(ADD_SUB_ISSUE, issue, parent)
    }

    fn add_relates_to(&self, issue: &ItemRef, other: &ItemRef) -> Result<(), Error> {
        self.link(ADD_RELATES_TO, issue, other)
    }

    fn close(&self, issue: &ItemRef, comment: &str) -> Result<(), Error> {
        self.gh(
            &["issue", "close", &issue.to_string(), "--comment", comment],
            None,
        )
        .map(drop)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ut_a_url_names_its_own_repository() {
        let r = |s: &str| repo_of(&ItemRef::parse(s).unwrap());
        assert_eq!(r("12").unwrap(), "{owner}/{repo}");
        assert_eq!(r("https://github.com/o2/r.2/pull/12").unwrap(), "o2/r.2");
        assert_eq!(r("https://github.com/o/r/issues/3/").unwrap(), "o/r");
        for bad in [
            "https://github.com/o/pull/12",
            "https://github.com/../r/pull/12",
            "https://github.com/o/r/x/pull/12",
        ] {
            assert!(r(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn ut_ids_and_branches_reaching_gh_are_checked() {
        assert!(thread_id("PRRT_kwDOAbc-_=").is_ok());
        for bad in ["", "@/etc/passwd", "a b", "x;y", "T1\n"] {
            assert!(thread_id(bad).is_err(), "{bad}");
        }
        assert!(run_id("123").is_ok());
        for bad in ["", "-x", "12a", "1 2"] {
            assert!(run_id(bad).is_err(), "{bad}");
        }
        assert!(ref_path("feat/x-1.2_b+c@d").is_ok());
        for bad in ["", "-x", "a..b", "a?b", "a#b", "a%2fb", "a b"] {
            assert!(ref_path(bad).is_err(), "{bad}");
        }
    }

    fn thread(id: &str, resolved: bool, n: usize, more: Option<&str>) -> String {
        let comments: Vec<String> = (0..n)
            .map(|i| format!(r#"{{"databaseId":{i},"author":null,"createdAt":"t","body":"c{i}"}}"#))
            .collect();
        let info = more.map_or(
            r#"{"hasNextPage":false,"endCursor":null}"#.to_owned(),
            |c| format!(r#"{{"hasNextPage":true,"endCursor":"{c}"}}"#),
        );
        format!(
            r#"{{"id":"{id}","isResolved":{resolved},"path":"a.rs","line":1,"originalLine":1,"comments":{{"pageInfo":{info},"nodes":[{}]}}}}"#,
            comments.join(",")
        )
    }

    fn page(threads: &[String], next: Option<&str>) -> String {
        let info = next.map_or(
            r#"{"hasNextPage":false,"endCursor":null}"#.to_owned(),
            |c| format!(r#"{{"hasNextPage":true,"endCursor":"{c}"}}"#),
        );
        format!(
            r#"{{"data":{{"repository":{{"pullRequest":{{"state":"OPEN","isDraft":true,"reviewThreads":{{"pageInfo":{info},"nodes":[{}]}}}}}}}}}}"#,
            threads.join(",")
        )
    }

    #[test]
    fn ut_github_threads_follow_both_cursors() {
        let mut thread_calls = Vec::new();
        let mut comment_calls = Vec::new();
        let (state, draft, all) = threads(
            |after| {
                thread_calls.push(after.map(str::to_owned));
                Ok(match after {
                    None => page(&[thread("T1", true, 1, Some("K1"))], Some("P1")),
                    _ => page(&[thread("T2", false, 1, None)], None),
                })
            },
            |id, after| {
                comment_calls.push((id.to_owned(), after.to_owned()));
                Ok(r#"{"data":{"node":{"comments":{"pageInfo":{"hasNextPage":false,"endCursor":null},"nodes":[{"databaseId":9,"author":null,"createdAt":"t","body":"late"}]}}}}"#.to_owned())
            },
        )
        .unwrap();
        assert_eq!((state.as_str(), draft), ("OPEN", true));
        assert_eq!(
            all.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
            ["T1", "T2"]
        );
        assert_eq!(all[0].comments.nodes.len(), 2);
        assert_eq!(thread_calls, [None, Some("P1".to_owned())]);
        assert_eq!(comment_calls, [("T1".to_owned(), "K1".to_owned())]);
    }

    #[test]
    fn ut_github_threads_refuse_a_missing_cursor() {
        let bad = r#"{"data":{"repository":{"pullRequest":{"state":"OPEN","isDraft":false,"reviewThreads":{"pageInfo":{"hasNextPage":true,"endCursor":null},"nodes":[]}}}}}"#;
        let e = threads(|_| Ok(bad.to_owned()), |_, _| unreachable!());
        assert!(matches!(e, Err(Error::Shape(_))));
    }

    #[test]
    fn ut_no_checks() {
        assert!(no_checks("no checks reported on the 'feat/x' branch\n"));
        assert!(!no_checks("build\tfail\t1m\n"));
    }

    #[test]
    fn ut_paths_refuse_keys_and_odd_comment_ids() {
        let gh = Gh::new(Path::new("/nonexistent"));
        let key = ItemRef::Key("KEY-1".into());
        assert!(matches!(gh.branch(&key), Err(Error::Ref(_))));
        assert!(matches!(CodeHost::view(&gh, &key), Err(Error::Ref(_))));
        assert!(matches!(
            gh.reply(&ItemRef::Number(1), "1/../../x", "t"),
            Err(Error::Ref(_))
        ));
    }

    const LINKS_JSON: &str = r#"{"data":{"repository":{"nameWithOwner":"o/r","issue":{
        "state":"OPEN",
        "milestone":{"number":3,"title":"Epic","description":null,"state":"OPEN"},
        "labels":{"nodes":[{"name":"area:runs"},{"name":"bug"}]},
        "parent":{"number":2,"url":"https://github.com/o/r/issues/2","repository":{"nameWithOwner":"o/r"}},
        "blockedBy":{"nodes":[
            {"number":7,"url":"https://github.com/o/r/issues/7","state":"OPEN","repository":{"nameWithOwner":"o/r"}},
            {"number":8,"url":"https://github.com/o/r/issues/8","state":"CLOSED","repository":{"nameWithOwner":"o/r"}}]},
        "issueDependenciesSummary":{"blockedBy":1},
        "relatesTo":{"nodes":[{"number":9,"url":"https://github.com/o/r/issues/9","repository":{"nameWithOwner":"o/r"}}]}
    }}}}"#;

    /// Pages from `--slurp` flatten, and a null description reads as empty.
    #[test]
    fn ut_milestone_pages_flatten_and_read_a_null_description() {
        let text = r#"[[{"number":1,"title":"A","description":null,"state":"open"}],
            [{"number":2,"title":"B","description":"text","state":"closed"}]]"#;
        let got = milestones_of(text).unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].description, "");
        assert!(got[0].open);
        assert_eq!(got[1].title, "B");
        assert_eq!(got[1].description, "text");
        assert!(!got[1].open);
    }

    /// Pull requests are dropped and the issues come ranked by number.
    #[test]
    fn ut_issues_in_drops_pull_requests_and_sorts() {
        let text = r#"[[{"number":9,"title":"c"},{"number":4,"title":"pr","pull_request":{}}],
            [{"number":2,"title":"a"}]]"#;
        let got = issues_of(text).unwrap();
        assert_eq!(got, vec![(2, "a".to_owned()), (9, "c".to_owned())]);
    }

    /// Links carry the milestone, labels, parent, blockers and relations.
    #[test]
    fn ut_links_maps_relationships() {
        let l = links_of(LINKS_JSON).unwrap();
        assert!(l.open);
        let m = l.milestone.unwrap();
        assert_eq!((m.number, m.description.as_str(), m.open), (3, "", true));
        assert_eq!(l.labels, vec!["area:runs", "bug"]);
        assert_eq!(l.parent, Some(ItemRef::Number(2)));
        assert_eq!(l.blocked_by.len(), 2);
        assert_eq!(l.blocked_by[0].cite, "#7");
        assert_eq!(l.blocked_by[0].reference, ItemRef::Number(7));
        assert!(l.blocked_by[0].open);
        assert!(!l.blocked_by[1].open);
        assert_eq!(l.relates_to, vec![ItemRef::Number(9)]);
    }

    /// An item of another repository is a URL and cites its repository.
    #[test]
    fn ut_links_cites_another_repository() {
        let text = LINKS_JSON.replace(
            r#"{"number":7,"url":"https://github.com/o/r/issues/7","state":"OPEN","repository":{"nameWithOwner":"o/r"}}"#,
            r#"{"number":7,"url":"https://github.com/x/y/issues/7","state":"OPEN","repository":{"nameWithOwner":"x/y"}}"#,
        );
        let l = links_of(&text).unwrap();
        assert_eq!(l.blocked_by[0].cite, "x/y#7");
        assert_eq!(
            l.blocked_by[0].reference,
            ItemRef::Url {
                url: "https://github.com/x/y/issues/7".to_owned(),
                number: 7
            }
        );
    }

    /// Blockers GitHub counts open but does not list are `unreadable`.
    #[test]
    fn ut_unreadable_counts_hidden_open_blockers() {
        assert_eq!(links_of(LINKS_JSON).unwrap().unreadable, 0);
        let hidden = LINKS_JSON.replace(r#""blockedBy":1"#, r#""blockedBy":4"#);
        assert_eq!(links_of(&hidden).unwrap().unreadable, 3);
        let fewer = LINKS_JSON.replace(r#""blockedBy":1"#, r#""blockedBy":0"#);
        assert_eq!(links_of(&fewer).unwrap().unreadable, 0);
    }

    /// RU-E-010 — a 422 on a description write is rejected with its length and message.
    #[test]
    fn ut_a_422_on_a_description_is_rejected() {
        let e = Error::Command("gh: Validation Failed (HTTP 422)".to_owned());
        match rejected(e, "héllo") {
            Error::Rejected { length, detail } => {
                assert_eq!(length, 5);
                assert!(detail.contains("HTTP 422"));
            }
            other => panic!("{other:?}"),
        }
    }

    /// Any other failure passes through unchanged.
    #[test]
    fn ut_other_errors_are_not_rejected() {
        let e = rejected(Error::Command("gh: HTTP 500".to_owned()), "x");
        assert!(matches!(e, Error::Command(m) if m == "gh: HTTP 500"));
        let e = rejected(Error::NotFound("x".to_owned()), "x");
        assert!(matches!(e, Error::NotFound(_)));
    }

    /// RU-R-041 — a label ferrule creates carries a color of six hex digits.
    #[test]
    fn ut_new_label_carries_a_color() {
        let args = new_label_args("area:runs", "1a2b3c");
        assert!(args.contains(&"name=area:runs".to_owned()), "{args:?}");
        assert!(args.contains(&"color=1a2b3c".to_owned()), "{args:?}");
        let c = random_color();
        assert_eq!(c.len(), 6, "{c}");
        assert!(c.bytes().all(|b| b.is_ascii_hexdigit()), "{c}");
    }

    /// RU-R-041 — the colors differ from one label to the next.
    #[test]
    fn ut_label_colors_vary() {
        let all: std::collections::HashSet<String> = (0..16).map(|_| random_color()).collect();
        assert!(all.len() > 1, "{all:?}");
    }

    /// RU-R-042 — a label the repository has, under another case too, is left as it is.
    #[test]
    fn ut_labels_match_without_case() {
        let have = vec!["Area:Runs".to_owned()];
        let want = vec!["area:runs".to_owned(), "area:card".to_owned()];
        assert_eq!(missing_labels(&have, &want), vec!["area:card"]);
    }
}
