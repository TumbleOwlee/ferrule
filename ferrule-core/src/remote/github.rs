//! GitHub as the code host and the issue tracker, through `gh`. The explicit `--json` field
//! lists keep `gh` off the deprecated Projects-Classic query.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use super::host::{Checks, CodeHost, PrState, PrStatus, comment_id};
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
}
