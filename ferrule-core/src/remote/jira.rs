//! Jira as the issue tracker, through `curl` with the credentials of
//! `.ferrule/jira.local.json`, read once on first use. Only reading an
//! issue is implemented; filing, editing, commenting and searching answer
//! [`Error::Unsupported`].

use std::cell::OnceCell;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::Value;

use super::tracker::{Found, IssueTracker, SearchState};
use super::{Comment, Error, Item, ItemRef, MAX_PAGES, creds, curl, json};

/// An Atlassian document as plain text, one line per paragraph, heading,
/// list item and code line, so the line-based readers (`issue request`)
/// find their lines: a hard break starts a new line, a list item is
/// `- `, a mention or emoji is its text.
fn adf_text(v: &Value) -> String {
    const BLOCKS: &[&str] = &[
        "paragraph",
        "heading",
        "listItem",
        "codeBlock",
        "blockquote",
        "rule",
        "panel",
        "tableRow",
        "mediaSingle",
    ];
    fn walk(v: &Value, out: &mut String) {
        match v {
            Value::Object(m) => {
                let kind = m.get("type").and_then(Value::as_str).unwrap_or("");
                match kind {
                    "text" => out.push_str(m.get("text").and_then(Value::as_str).unwrap_or("")),
                    "hardBreak" => out.push('\n'),
                    "mention" | "emoji" => {
                        out.push_str(m["attrs"]["text"].as_str().unwrap_or(""));
                    }
                    _ => {
                        if kind == "listItem" {
                            out.push_str("- ");
                        }
                        if let Some(c) = m.get("content") {
                            walk(c, out);
                        }
                        if BLOCKS.contains(&kind) && !out.is_empty() && !out.ends_with('\n') {
                            out.push('\n');
                        }
                    }
                }
            }
            Value::Array(a) => a.iter().for_each(|x| walk(x, out)),
            _ => {}
        }
    }
    let mut out = String::new();
    walk(v, &mut out);
    out.trim_end().to_owned()
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct JiraCreds {
    base_url: String,
    email: String,
    api_token: String,
}

/// Whether a Jira status is not closed: its category is not `done`, whatever
/// the project names its statuses.
fn jira_open(status: &Value) -> bool {
    status["statusCategory"]["key"] != "done"
}

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

pub struct Jira {
    root: PathBuf,
    /// The credentials, or why they could not be read.
    creds: OnceCell<Result<JiraCreds, String>>,
}

impl Jira {
    #[must_use]
    pub fn new(root: &Path) -> Self {
        Jira {
            root: root.to_owned(),
            creds: OnceCell::new(),
        }
    }

    fn creds(&self) -> Result<&JiraCreds, Error> {
        self.creds
            .get_or_init(|| {
                creds(&self.root, "jira.local.json").map_err(|e| match e {
                    Error::Credentials(m) => m,
                    other => other.to_string(),
                })
            })
            .as_ref()
            .map_err(|m| Error::Credentials(m.clone()))
    }

    fn get(&self, path: &str) -> Result<Value, Error> {
        let c = self.creds()?;
        let url = format!("{}/rest/api/3/{path}", c.base_url);
        json(&curl(&c.email, &c.api_token, &[&url], &self.root)?, "jira")
    }

    fn unsupported(&self, op: &'static str) -> Error {
        Error::Unsupported {
            host: IssueTracker::name(self),
            op,
        }
    }
}

impl IssueTracker for Jira {
    fn name(&self) -> &'static str {
        "Jira"
    }

    fn view(&self, issue: &ItemRef) -> Result<Item, Error> {
        let key = issue.segment();
        let i = self.get(&format!(
            "issue/{key}?fields=summary,status,reporter,description"
        ))?;
        let comments = jira_comments(|start| {
            self.get(&format!(
                "issue/{key}/comment?orderBy=created&startAt={start}&maxResults=100"
            ))
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

    fn create(&self, _title: &str, _body: &str) -> Result<ItemRef, Error> {
        Err(self.unsupported("issue create"))
    }

    fn edit(&self, _issue: &ItemRef, _title: &str, _body: &str) -> Result<(), Error> {
        Err(self.unsupported("issue edit"))
    }

    fn comment(&self, _issue: &ItemRef, _body: &str) -> Result<(), Error> {
        Err(self.unsupported("issue comment"))
    }

    fn find_open(&self, _title: &str) -> Result<Option<ItemRef>, Error> {
        Err(self.unsupported("issue search"))
    }

    fn search(&self, _: &[String], _: SearchState, _: usize) -> Result<Vec<Found>, Error> {
        Err(self.unsupported("issue search"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn ut_jira_closed_by_status_category() {
        let st = |c: &str| serde_json::json!({ "name": "Shipped", "statusCategory": { "key": c } });
        assert!(!jira_open(&st("done")));
        assert!(jira_open(&st("indeterminate")));
        assert!(jira_open(&st("new")));
    }

    #[test]
    fn ut_adf_text_keeps_lines() {
        let v: Value = serde_json::from_str(
            r#"{"type":"doc","content":[
                {"type":"heading","content":[{"type":"text","text":"Goal"}]},
                {"type":"paragraph","content":[{"type":"text","text":"a "},{"type":"text","text":"b","marks":[{"type":"strong"}]},{"type":"hardBreak"},{"type":"text","text":"c"}]},
                {"type":"bulletList","content":[
                    {"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"one"}]}]},
                    {"type":"listItem","content":[{"type":"paragraph","content":[{"type":"mention","attrs":{"text":"@al"}}]}]}
                ]}
            ]}"#,
        )
        .unwrap();
        assert_eq!(adf_text(&v), "Goal\na b\nc\n- one\n- @al");
    }

    #[test]
    fn ut_jira_writes_unsupported_without_reading_credentials() {
        let jira = Jira::new(Path::new("/nonexistent"));
        let e = jira.create("t", "b").unwrap_err();
        assert!(matches!(
            e,
            Error::Unsupported {
                host: "Jira",
                op: "issue create"
            }
        ));
        assert_eq!(
            e.to_string(),
            "issue create is only implemented for GitHub yet"
        );
        assert!(matches!(
            jira.view(&ItemRef::Key("KEY-1".into())),
            Err(Error::Credentials(_))
        ));
    }
}
