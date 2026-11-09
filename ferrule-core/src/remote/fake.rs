//! An in-process [`CodeHost`] and [`IssueTracker`] for unit tests: one
//! pull request and a few open issues held in memory, every call recorded.

use std::cell::RefCell;
use std::rc::Rc;

use super::host::{Checks, CodeHost, PrStatus};
use std::collections::HashMap;

use super::milestones::{IssueLinks, Milestone, Milestones, NewIssue};
use super::tracker::{Found, IssueTracker};
use super::{Error, Item, ItemRef, Query, SearchState};
use super::{Fetched, Thread};

/// What the fake serves, and the calls it received.
#[derive(Debug, Default)]
pub(crate) struct Served {
    /// The open PR `find_open` answers.
    pub open: Option<u64>,
    /// The number `create_draft` returns.
    pub next: u64,
    /// The PR's body, title, state and draft flag.
    pub body: String,
    pub title: String,
    pub state: String,
    pub draft: bool,
    pub threads: Vec<Thread>,
    /// One line per call: the operation and its PR or branch.
    pub calls: Vec<String>,
}

/// A handle on shared [`Served`]: the test keeps a clone to read the calls.
#[derive(Debug, Clone, Default)]
pub(crate) struct FakeHost(pub Rc<RefCell<Served>>);

impl FakeHost {
    pub(crate) fn calls(&self) -> Vec<String> {
        self.0.borrow().calls.clone()
    }

    fn call(&self, line: String) {
        self.0.borrow_mut().calls.push(line);
    }
}

impl CodeHost for FakeHost {
    fn name(&self) -> &'static str {
        "Fake"
    }

    fn view(&self, pr: &ItemRef) -> Result<Item, Error> {
        self.call(format!("view {pr}"));
        let s = self.0.borrow();
        Ok(Item {
            head: format!("#{pr} {} [{}]", s.title, s.state.to_uppercase()),
            url: None,
            open: s.state == "open",
            by: "fake".to_owned(),
            body: s.body.clone(),
            comments: Vec::new(),
        })
    }

    fn branch(&self, pr: &ItemRef) -> Result<String, Error> {
        self.call(format!("branch {pr}"));
        Ok("feat/x".to_owned())
    }

    fn find_open(&self, branch: &str) -> Result<Option<u64>, Error> {
        self.call(format!("find_open {branch}"));
        Ok(self.0.borrow().open)
    }

    fn create_draft(
        &self,
        branch: &str,
        base: &str,
        title: &str,
        body: &str,
    ) -> Result<u64, Error> {
        self.call(format!("create_draft {branch} {base}"));
        let mut s = self.0.borrow_mut();
        title.clone_into(&mut s.title);
        body.clone_into(&mut s.body);
        "open".clone_into(&mut s.state);
        s.draft = true;
        s.open = Some(s.next);
        Ok(s.next)
    }

    fn edit(&self, pr: &ItemRef, title: Option<&str>, body: &str) -> Result<(), Error> {
        self.call(format!("edit {pr}"));
        let mut s = self.0.borrow_mut();
        if let Some(t) = title {
            t.clone_into(&mut s.title);
        }
        body.clone_into(&mut s.body);
        Ok(())
    }

    fn ready(&self, pr: &ItemRef) -> Result<(), Error> {
        self.call(format!("ready {pr}"));
        self.0.borrow_mut().draft = false;
        Ok(())
    }

    fn status(&self, pr: &ItemRef) -> Result<PrStatus, Error> {
        self.call(format!("status {pr}"));
        let s = self.0.borrow();
        Ok(PrStatus {
            state: super::host::PrState::parse(&s.state),
            draft: s.draft,
            body: s.body.clone(),
        })
    }

    fn checks_watch(&self, pr: &ItemRef) -> Result<Checks, Error> {
        self.call(format!("checks_watch {pr}"));
        Ok(Checks {
            code: 0,
            failed: Vec::new(),
            raw: String::new(),
        })
    }

    fn threads(&self, pr: &ItemRef) -> Result<Fetched, Error> {
        self.call(format!("threads {pr}"));
        let s = self.0.borrow();
        Ok(Fetched {
            state: s.state.clone(),
            draft: s.draft,
            threads: s.threads.clone(),
        })
    }

    fn reply(&self, pr: &ItemRef, comment: &str, _text: &str) -> Result<(), Error> {
        self.call(format!("reply {pr} {comment}"));
        Ok(())
    }

    fn resolve(&self, pr: &ItemRef, thread: &str, _comment: &str) -> Result<(), Error> {
        self.call(format!("resolve {pr} {thread}"));
        self.0.borrow_mut().threads.retain(|t| t.id != thread);
        Ok(())
    }

    fn failed_log(&self, branch: &str, _query: Query) -> Result<String, Error> {
        self.call(format!("failed_log {branch}"));
        Err(Error::NotFound(format!(
            "No failed run found for branch '{branch}'"
        )))
    }
}

/// What the fake tracker serves, and the calls it received.
#[derive(Debug, Default)]
#[allow(clippy::struct_field_names)]
pub(crate) struct Issues {
    /// The open issues `find_open` searches: number and title.
    pub open: Vec<(u64, String)>,
    /// The number the next `create` returns.
    pub next: u64,
    /// Every write and search answers [`Error::Unsupported`], as Jira's do.
    pub read_only: bool,
    /// One line per call: the operation and its issue or title.
    pub calls: Vec<String>,
    /// Comments posted: the issue and the body.
    pub comments: Vec<(u64, String)>,
    /// The milestones, numbered from `next` in creation order.
    pub milestones: Vec<Milestone>,
    /// What `links` answers, by issue.
    pub links: HashMap<u64, IssueLinks>,
    /// The issues of each milestone, by milestone number.
    pub issues_in: HashMap<u64, Vec<u64>>,
    /// `milestones` answers `None`, as a tracker without them does.
    pub no_milestones: bool,
    /// A write whose call line starts with this fails after it is recorded.
    pub fail_on: Option<&'static str>,
    /// The login `login` answers.
    pub login: String,
    /// Issue bodies: what `view` serves, `edit` and `create_issue` store.
    pub bodies: HashMap<u64, String>,
}

/// An in-process [`IssueTracker`]: a handle on shared [`Issues`].
#[derive(Debug, Clone, Default)]
pub(crate) struct FakeTracker(pub Rc<RefCell<Issues>>);

impl FakeTracker {
    pub(crate) fn calls(&self) -> Vec<String> {
        self.0.borrow().calls.clone()
    }

    /// Records the call; `Err` when the tracker is read-only.
    fn write(&self, op: &'static str, line: String) -> Result<(), Error> {
        let mut s = self.0.borrow_mut();
        s.calls.push(line);
        let line = s.calls.last().cloned().unwrap_or_default();
        if s.read_only || s.fail_on.is_some_and(|p| line.starts_with(p)) {
            return Err(Error::Unsupported { host: "Fake", op });
        }
        Ok(())
    }
}

impl IssueTracker for FakeTracker {
    fn name(&self) -> &'static str {
        "Fake"
    }

    fn view(&self, issue: &ItemRef) -> Result<Item, Error> {
        self.0.borrow_mut().calls.push(format!("view {issue}"));
        let s = self.0.borrow();
        let open = s.open.iter().find(|(n, _)| issue.number() == Some(*n));
        Ok(Item {
            head: format!(
                "#{issue} {} [{}]",
                open.map_or("", |(_, t)| t.as_str()),
                if open.is_some() { "OPEN" } else { "CLOSED" }
            ),
            url: None,
            open: open.is_some(),
            by: "fake".to_owned(),
            body: issue
                .number()
                .and_then(|n| s.bodies.get(&n).cloned())
                .unwrap_or_default(),
            comments: s
                .comments
                .iter()
                .filter(|(n, _)| issue.number() == Some(*n))
                .map(|(_, body)| crate::remote::Comment {
                    author: "fake".to_owned(),
                    at: String::new(),
                    body: body.clone(),
                })
                .collect(),
        })
    }

    fn create(&self, title: &str, _body: &str) -> Result<ItemRef, Error> {
        self.write("issue create", format!("create {title}"))?;
        let mut s = self.0.borrow_mut();
        let n = s.next;
        s.open.push((n, title.to_owned()));
        s.next += 1;
        Ok(ItemRef::Number(n))
    }

    fn edit(&self, issue: &ItemRef, title: &str, body: &str) -> Result<(), Error> {
        self.write("issue edit", format!("edit {issue} {title}"))?;
        if let Some(n) = issue.number() {
            self.0.borrow_mut().bodies.insert(n, body.to_owned());
        }
        Ok(())
    }

    fn comment(&self, issue: &ItemRef, body: &str) -> Result<(), Error> {
        self.write("issue comment", format!("comment {issue}"))?;
        if let Some(n) = issue.number() {
            self.0.borrow_mut().comments.push((n, body.to_owned()));
        }
        Ok(())
    }

    fn find_open(&self, title: &str) -> Result<Option<ItemRef>, Error> {
        self.write("issue search", format!("find_open {title}"))?;
        Ok(self
            .0
            .borrow()
            .open
            .iter()
            .find(|(_, t)| t == title)
            .map(|(n, _)| ItemRef::Number(*n)))
    }

    fn search(
        &self,
        keywords: &[String],
        state: super::SearchState,
        _limit: usize,
    ) -> Result<Vec<Found>, Error> {
        self.write(
            "issue search",
            format!("search {} {}", keywords.join(" "), state.as_str()),
        )?;
        Ok(Vec::new())
    }

    fn milestones(&self) -> Option<&dyn Milestones> {
        if self.0.borrow().no_milestones {
            None
        } else {
            Some(self)
        }
    }
}

/// `key=value`, built apart so the line is no status token.
fn pair(key: &str, value: &str) -> String {
    format!("{key}={value}")
}

fn joined(labels: &[String]) -> String {
    labels.join(",")
}

impl Milestones for FakeTracker {
    fn login(&self) -> Result<String, Error> {
        let mut s = self.0.borrow_mut();
        s.calls.push("login".to_owned());
        Ok(s.login.clone())
    }

    fn list(&self, state: SearchState) -> Result<Vec<Milestone>, Error> {
        let mut s = self.0.borrow_mut();
        s.calls.push(format!("list {}", state.as_str()));
        Ok(s.milestones
            .iter()
            .filter(|m| match state {
                SearchState::Open => m.open,
                SearchState::Closed => !m.open,
                SearchState::All => true,
            })
            .cloned()
            .collect())
    }

    fn get(&self, number: u64) -> Result<Milestone, Error> {
        let mut s = self.0.borrow_mut();
        s.calls.push(format!("get {number}"));
        s.milestones
            .iter()
            .find(|m| m.number == number)
            .cloned()
            .ok_or_else(|| Error::NotFound(format!("no milestone #{number}")))
    }

    fn create(&self, title: &str, description: &str) -> Result<Milestone, Error> {
        self.write("milestone create", format!("create_milestone {title}"))?;
        let mut s = self.0.borrow_mut();
        let m = Milestone {
            number: s.next,
            title: title.to_owned(),
            description: description.to_owned(),
            open: true,
        };
        s.next += 1;
        s.milestones.push(m.clone());
        Ok(m)
    }

    fn set_description(&self, number: u64, description: &str) -> Result<(), Error> {
        self.write("milestone edit", format!("set_description {number}"))?;
        let mut s = self.0.borrow_mut();
        let m = s
            .milestones
            .iter_mut()
            .find(|m| m.number == number)
            .ok_or_else(|| Error::NotFound(format!("no milestone #{number}")))?;
        description.clone_into(&mut m.description);
        Ok(())
    }

    fn issues_in(&self, number: u64) -> Result<Vec<u64>, Error> {
        let mut s = self.0.borrow_mut();
        s.calls.push(format!("issues_in {number}"));
        let mut v = s.issues_in.get(&number).cloned().unwrap_or_default();
        v.sort_unstable();
        Ok(v)
    }

    fn find_open_in(&self, milestone: u64, title: &str) -> Result<Option<ItemRef>, Error> {
        let mut s = self.0.borrow_mut();
        s.calls.push(format!("find_open_in {milestone} {title}"));
        let inside = s.issues_in.get(&milestone);
        Ok(s.open
            .iter()
            .find(|(n, t)| t == title && inside.is_some_and(|v| v.contains(n)))
            .map(|(n, _)| ItemRef::Number(*n)))
    }

    fn links(&self, issue: &ItemRef) -> Result<IssueLinks, Error> {
        let mut s = self.0.borrow_mut();
        s.calls.push(format!("links {issue}"));
        Ok(issue
            .number()
            .and_then(|n| s.links.get(&n).cloned())
            .unwrap_or_default())
    }

    fn create_issue(&self, new: &NewIssue<'_>) -> Result<ItemRef, Error> {
        let number = {
            let s = self.0.borrow();
            new.milestone
                .and_then(|t| s.milestones.iter().find(|m| m.title == t))
                .map(|m| m.number)
        };
        self.write(
            "issue create",
            format!(
                "create_issue {} {} {}",
                new.title,
                pair(
                    "milestone",
                    &number.map_or_else(|| "-".to_owned(), |n| n.to_string())
                ),
                pair("labels", &joined(new.labels))
            ),
        )?;
        let mut s = self.0.borrow_mut();
        let n = s.next;
        s.open.push((n, new.title.to_owned()));
        s.bodies.insert(n, new.body.to_owned());
        s.next += 1;
        if let Some(m) = number {
            s.issues_in.entry(m).or_default().push(n);
        }
        Ok(ItemRef::Number(n))
    }

    fn set_milestone(&self, issue: &ItemRef, title: &str) -> Result<(), Error> {
        self.write("issue edit", format!("set_milestone {issue} {title}"))
    }

    fn ensure_labels(&self, labels: &[String]) -> Result<(), Error> {
        self.write("label create", format!("ensure_labels {}", joined(labels)))
    }

    fn add_labels(&self, issue: &ItemRef, labels: &[String]) -> Result<(), Error> {
        self.write(
            "issue edit",
            format!("add_labels {issue} {}", joined(labels)),
        )
    }

    fn add_blocked_by(&self, issue: &ItemRef, blocker: &ItemRef) -> Result<(), Error> {
        self.write("issue edit", format!("add_blocked_by {issue} {blocker}"))
    }

    fn set_parent(&self, issue: &ItemRef, parent: &ItemRef) -> Result<(), Error> {
        self.write("issue edit", format!("set_parent {issue} {parent}"))
    }

    fn add_relates_to(&self, issue: &ItemRef, other: &ItemRef) -> Result<(), Error> {
        self.write("issue edit", format!("add_relates_to {issue} {other}"))
    }

    fn close(&self, issue: &ItemRef, _comment: &str) -> Result<(), Error> {
        self.write("issue close", format!("close {issue}"))?;
        if let Some(n) = issue.number() {
            self.0.borrow_mut().open.retain(|(o, _)| *o != n);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::milestones::Milestones;
    use super::*;

    /// A tracker that does not override `milestones` has none.
    #[test]
    fn ut_a_tracker_has_no_milestones_by_default() {
        struct Bare;
        impl IssueTracker for Bare {
            fn name(&self) -> &'static str {
                "Bare"
            }
            fn view(&self, _: &ItemRef) -> Result<Item, Error> {
                Err(Error::NotFound(String::new()))
            }
            fn create(&self, _: &str, _: &str) -> Result<ItemRef, Error> {
                Err(Error::NotFound(String::new()))
            }
            fn edit(&self, _: &ItemRef, _: &str, _: &str) -> Result<(), Error> {
                Ok(())
            }
            fn comment(&self, _: &ItemRef, _: &str) -> Result<(), Error> {
                Ok(())
            }
            fn find_open(&self, _: &str) -> Result<Option<ItemRef>, Error> {
                Ok(None)
            }
            fn search(
                &self,
                _: &[String],
                _: super::super::SearchState,
                _: usize,
            ) -> Result<Vec<Found>, Error> {
                Ok(Vec::new())
            }
        }
        assert!(Bare.milestones().is_none());
    }

    /// The fake answers `Some` until told otherwise, then `None`.
    #[test]
    fn ut_fake_tracker_answers_none_when_told() {
        let t = FakeTracker::default();
        assert!(t.milestones().is_some());
        t.0.borrow_mut().no_milestones = true;
        assert!(t.milestones().is_none());
    }

    /// A rejected description refuses with its length and GitHub's message.
    #[test]
    fn ut_rejected_refusal_names_length_and_detail() {
        let e = Error::Rejected {
            length: 9,
            detail: "too long".to_owned(),
        };
        let line = e.refusal("post").to_string();
        assert!(line.contains("rejected length=9"), "{line}");
        assert!(line.contains("detail=\"too long\""), "{line}");
        assert!(line.contains("hint="), "{line}");
    }

    /// The fake records milestone calls and files issues into a milestone.
    #[test]
    fn ut_fake_files_an_issue_into_its_milestone() {
        let t = FakeTracker::default();
        t.0.borrow_mut().next = 5;
        let m = Milestones::create(&t, "Epic", "brief").unwrap();
        assert_eq!(m.number, 5);
        let i = t
            .create_issue(&super::super::NewIssue {
                title: "s1",
                body: "b",
                milestone: Some("Epic"),
                labels: &["area:runs".to_owned()],
            })
            .unwrap();
        assert_eq!(i, ItemRef::Number(6));
        assert_eq!(t.issues_in(5).unwrap(), vec![6]);
        assert_eq!(
            t.calls(),
            vec![
                "create_milestone Epic",
                "create_issue s1 milestone=5 labels=area:runs",
                "issues_in 5"
            ]
        );
    }
}
