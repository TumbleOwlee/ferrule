//! Bitbucket as the code host, through `curl` with the credentials of
//! `.ferrule/bitbucket.local.json`, read once on first use. Opening,
//! editing and readying pull requests and watching their checks are not
//! implemented.

use std::cell::OnceCell;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::Value;

use super::host::{Checks, CodeHost, PrState, PrStatus, comment_id};
use super::{
    Comment, Error, Item, ItemRef, Query, creds, curl, follow_next, json, non_empty, pick_job,
};
use crate::pr::feedback::{Fetched, Thread};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BitbucketCreds {
    workspace: String,
    repo_slug: String,
    username: String,
    app_password: String,
}

pub struct Bitbucket {
    root: PathBuf,
    /// The credentials, or why they could not be read.
    creds: OnceCell<Result<BitbucketCreds, String>>,
}

impl Bitbucket {
    #[must_use]
    pub fn new(root: &Path) -> Self {
        Bitbucket {
            root: root.to_owned(),
            creds: OnceCell::new(),
        }
    }

    fn creds(&self) -> Result<&BitbucketCreds, Error> {
        self.creds
            .get_or_init(|| {
                creds(&self.root, "bitbucket.local.json").map_err(|e| match e {
                    Error::Credentials(m) => m,
                    other => other.to_string(),
                })
            })
            .as_ref()
            .map_err(|m| Error::Credentials(m.clone()))
    }

    /// The repository's API URL with `path` appended; `path` is built from
    /// numbers and constants only.
    fn api(&self, path: &str) -> Result<String, Error> {
        let c = self.creds()?;
        Ok(format!(
            "https://api.bitbucket.org/2.0/repositories/{}/{}/{path}",
            c.workspace, c.repo_slug
        ))
    }

    /// The pull request's API URL. A URL naming another workspace or
    /// repository is refused: the number would land on this one's.
    fn pr_api(&self, pr: &ItemRef) -> Result<String, Error> {
        if let ItemRef::Url { url, .. } = pr {
            let c = self.creds()?;
            let path = url.split_once("://").map_or(url.as_str(), |(_, r)| r);
            let path = path.split(['?', '#']).next().unwrap_or_default();
            let segments: Vec<&str> = path.trim_end_matches('/').split('/').collect();
            let ours = matches!(
                segments.as_slice(),
                [_host, ws, repo, "pull-requests", _n, ..]
                    if ws.eq_ignore_ascii_case(&c.workspace) && repo.eq_ignore_ascii_case(&c.repo_slug)
            );
            if !ours {
                return Err(Error::Ref(format!(
                    "{url} is not a pull request of {}/{}",
                    c.workspace, c.repo_slug
                )));
            }
        }
        self.api(&format!("pullrequests/{}", pr.pr_number()?))
    }

    fn call(&self, args: &[&str]) -> Result<String, Error> {
        let c = self.creds()?;
        curl(&c.username, &c.app_password, args, &self.root)
    }

    fn get(&self, url: &str) -> Result<Value, Error> {
        json(&self.call(&[url])?, "bitbucket")
    }

    /// Every entry of a paged listing, all pages.
    fn values(&self, url: &str) -> Result<Vec<Value>, Error> {
        follow_next(url, |u| self.call(&[u]))
    }

    fn unsupported(&self, op: &'static str) -> Error {
        Error::Unsupported {
            host: self.name(),
            op,
        }
    }
}

/// The id of the top comment a reply hangs under, through any number of
/// replies to replies; a broken or looping chain ends where it breaks.
fn root_of<'a>(values: &[&'a Value], v: &'a Value) -> &'a Value {
    let mut at = v;
    for _ in 0..64 {
        let parent = &at["parent"]["id"];
        match values.iter().find(|c| c["id"] == *parent) {
            Some(p) if !parent.is_null() => at = p,
            _ => break,
        }
    }
    &at["id"]
}

/// A Bitbucket pull request's state (`open`, `merged`; declined and
/// superseded are `closed`), draft flag and description.
fn status(pr: &Value) -> PrStatus {
    PrStatus {
        state: PrState::parse(pr["state"].as_str().unwrap_or("")),
        draft: pr["draft"].as_bool().unwrap_or(false),
        body: pr["description"].as_str().unwrap_or("").to_owned(),
    }
}

/// A pipeline uuid (`{…}`) or number, as `ci failed --run` names one.
fn pipeline_id(given: &str) -> Result<&str, Error> {
    if !given.is_empty()
        && given
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '{' | '}' | '-'))
    {
        Ok(given)
    } else {
        Err(Error::Ref(format!(
            "'{given}' is not a pipeline uuid or number"
        )))
    }
}

impl CodeHost for Bitbucket {
    fn name(&self) -> &'static str {
        "Bitbucket"
    }

    fn view(&self, pr: &ItemRef) -> Result<Item, Error> {
        let api = self.pr_api(pr)?;
        let p = self.get(&api)?;
        let cs = self.values(&format!("{api}/comments?pagelen=50"))?;
        let s = |v: &Value, d: &str| v.as_str().unwrap_or(d).to_owned();
        let comments: Vec<Comment> = cs
            .iter()
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
                p["id"],
                s(&p["title"], ""),
                s(&p["state"], "")
            ),
            url: p["links"]["html"]["href"].as_str().map(str::to_owned),
            open: p["state"] == "OPEN",
            by: s(&p["author"]["display_name"], "unknown"),
            body: non_empty(p["description"].as_str(), "(no body)"),
            comments,
        })
    }

    fn branch(&self, pr: &ItemRef) -> Result<String, Error> {
        let p = self.get(&self.pr_api(pr)?)?;
        let branch = p["source"]["branch"]["name"].as_str().unwrap_or("");
        if branch.is_empty() {
            return Err(Error::NotFound(format!(
                "pull request {pr} has no head branch"
            )));
        }
        Ok(branch.to_owned())
    }

    fn find_open(&self, _branch: &str) -> Result<Option<u64>, Error> {
        Err(self.unsupported("pr list"))
    }

    fn create_draft(&self, _: &str, _: &str, _: &str, _: &str) -> Result<u64, Error> {
        Err(self.unsupported("pr create"))
    }

    fn edit(&self, _pr: &ItemRef, _title: Option<&str>, _body: &str) -> Result<(), Error> {
        Err(self.unsupported("pr edit"))
    }

    fn ready(&self, _pr: &ItemRef) -> Result<(), Error> {
        Err(self.unsupported("pr ready"))
    }

    fn status(&self, pr: &ItemRef) -> Result<PrStatus, Error> {
        Ok(status(&self.get(&self.pr_api(pr)?)?))
    }

    fn checks_watch(&self, _pr: &ItemRef) -> Result<Checks, Error> {
        Err(self.unsupported("pr checks"))
    }

    fn threads(&self, pr: &ItemRef) -> Result<Fetched, Error> {
        let api = self.pr_api(pr)?;
        let p = self.get(&api)?;
        let all = self.values(&format!("{api}/comments?pagelen=100"))?;
        let values: Vec<&Value> = all.iter().collect();
        let s = |v: &Value, d: &str| v.as_str().unwrap_or(d).to_owned();
        let comment = |v: &Value| {
            (
                s(&v["user"]["display_name"], "unknown"),
                s(&v["created_on"], ""),
                s(&v["content"]["raw"], "(empty)"),
            )
        };
        let threads = values
            .iter()
            .filter(|v| {
                !v["inline"].is_null()
                    && v["deleted"] != Value::Bool(true)
                    && v["parent"].is_null()
                    && v["resolution"].is_null()
            })
            .map(|r| {
                let id = r["id"].to_string();
                let line = [&r["inline"]["to"], &r["inline"]["from"]]
                    .into_iter()
                    .find(|v| !v.is_null())
                    .map_or_else(|| "?".to_owned(), ToString::to_string);
                let mut comments = vec![comment(r)];
                comments.extend(
                    values
                        .iter()
                        .filter(|v| {
                            !v["parent"].is_null()
                                && *root_of(&values, v) == r["id"]
                                && v["deleted"] != Value::Bool(true)
                        })
                        .map(|v| comment(v)),
                );
                Thread {
                    comment: id.clone(),
                    path: format!("{}:{line}", s(&r["inline"]["path"], "")),
                    id,
                    comments,
                }
            })
            .collect();
        Ok(Fetched {
            state: s(&p["state"], "").to_ascii_lowercase(),
            draft: p["draft"].as_bool().unwrap_or(false),
            threads,
        })
    }

    fn reply(&self, pr: &ItemRef, comment: &str, text: &str) -> Result<(), Error> {
        let api = format!("{}/comments", self.pr_api(pr)?);
        let parent: Value = comment
            .parse::<u64>()
            .map_or_else(|_| Value::String(comment.to_owned()), Value::from);
        let body = serde_json::json!({"content": {"raw": text}, "parent": {"id": parent}});
        self.call(&[
            "-H",
            "Content-Type: application/json",
            "-d",
            &body.to_string(),
            &api,
        ])
        .map(drop)
    }

    fn resolve(&self, pr: &ItemRef, _thread: &str, comment: &str) -> Result<(), Error> {
        let c = comment_id(comment)?;
        let api = format!("{}/comments/{c}/resolve", self.pr_api(pr)?);
        self.call(&["-X", "POST", &api]).map(drop)
    }

    fn failed_log(&self, branch: &str, query: Query) -> Result<String, Error> {
        let api = self.api("pipelines/")?;
        let uuid = if let Some(given) = query.run {
            pipeline_id(given)?.to_owned()
        } else {
            // BBQL string: a quote or backslash in the branch is escaped.
            let quoted = branch.replace('\\', "\\\\").replace('"', "\\\"");
            let q = format!("target.ref_name=\"{quoted}\" AND state.result.name=\"FAILED\"");
            let pipelines: Value = json(
                &self.call(&[
                    "-G",
                    &api,
                    "--data-urlencode",
                    "sort=-created_on",
                    "--data-urlencode",
                    "pagelen=1",
                    "--data-urlencode",
                    &format!("q={q}"),
                ])?,
                "bitbucket",
            )?;
            let Some(uuid) = pipelines["values"][0]["uuid"].as_str().map(str::to_owned) else {
                return Err(Error::NotFound(format!(
                    "No failed pipeline found for branch '{branch}'"
                )));
            };
            uuid
        };
        // Every page: a failing step can sit past the first.
        let steps = self.values(&format!("{api}{uuid}/steps/"))?;
        let failing: Vec<(&str, &str)> = steps
            .iter()
            .filter(|s| s["state"]["result"]["name"] == "FAILED")
            .filter_map(|s| Some((s["name"].as_str().unwrap_or(""), s["uuid"].as_str()?)))
            .collect();
        let step = pick_job(&failing, query, &uuid)?;
        self.call(&["-g", &format!("{api}{uuid}/steps/{step}/log")])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ut_a_reply_to_a_reply_belongs_to_the_top_comment() {
        let all: Vec<Value> = serde_json::from_str(
            r#"[{"id":1},{"id":2,"parent":{"id":1}},{"id":3,"parent":{"id":2}},{"id":4,"parent":{"id":9}},{"id":5,"parent":{"id":6}},{"id":6,"parent":{"id":5}}]"#,
        )
        .unwrap();
        let values: Vec<&Value> = all.iter().collect();
        let root = |i: usize| root_of(&values, values[i]).as_u64();
        assert_eq!(root(1), Some(1));
        assert_eq!(root(2), Some(1));
        assert_eq!(root(3), Some(4), "a missing parent ends the chain");
        assert!(root(4).is_some(), "a loop ends");
    }

    #[test]
    fn ut_bitbucket_pr_states() {
        let st =
            |s: &str| status(&serde_json::json!({ "state": s, "draft": true, "description": "d" }));
        assert_eq!(
            st("OPEN"),
            PrStatus {
                state: PrState::Open,
                draft: true,
                body: "d".to_owned()
            }
        );
        assert_eq!(st("MERGED").state, PrState::Merged);
        assert_eq!(st("DECLINED").state, PrState::Closed);
    }

    #[test]
    fn ut_unsupported_names_host_and_op() {
        let bb = Bitbucket::new(Path::new("/nonexistent"));
        let e = bb.ready(&ItemRef::Number(1)).unwrap_err();
        assert!(matches!(
            e,
            Error::Unsupported {
                host: "Bitbucket",
                op: "pr ready"
            }
        ));
        assert_eq!(e.to_string(), "pr ready is only implemented for GitHub yet");
        assert!(matches!(
            bb.status(&ItemRef::Number(1)),
            Err(Error::Credentials(_))
        ));
        assert!(pipeline_id("{ab-12}").is_ok());
        assert!(pipeline_id("../x").is_err());
    }
}
