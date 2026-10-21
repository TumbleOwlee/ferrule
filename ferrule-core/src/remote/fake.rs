//! An in-process [`CodeHost`] for unit tests: one pull request held in
//! memory, every call recorded.

use std::cell::RefCell;
use std::rc::Rc;

use super::host::{CodeHost, PrStatus};
use super::{Error, Item, ItemRef, Query};
use crate::pr::feedback::{Fetched, Thread};

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
            state: s.state.clone(),
            draft: s.draft,
            body: s.body.clone(),
        })
    }

    fn checks_watch(&self, pr: &ItemRef) -> Result<(i32, String), Error> {
        self.call(format!("checks_watch {pr}"));
        Ok((0, String::new()))
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
