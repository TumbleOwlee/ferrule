//! The code host: pull requests, their review threads and checks, and the
//! failed CI runs of a branch. Built once from the configuration; an
//! operation a host does not implement answers [`Error::Unsupported`].

use std::path::Path;

use super::bitbucket::Bitbucket;
use super::github::Gh;
use super::{Error, Item, ItemRef, Query, error_lines};
use crate::config::{Ci, Config};
use crate::pr::feedback::Fetched;

/// A pull request's state as `pr_status` reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrStatus {
    /// `open`, `merged` or `closed`.
    pub state: String,
    pub draft: bool,
    pub body: String,
}

/// The pull request operations of a code host.
pub trait CodeHost {
    /// The host's name, as an [`Error::Unsupported`] names it.
    fn name(&self) -> &'static str;

    /// A pull request with its comments.
    fn view(&self, pr: &ItemRef) -> Result<Item, Error>;

    /// A pull request's head branch.
    fn branch(&self, pr: &ItemRef) -> Result<String, Error>;

    /// The open pull request whose head is `branch`, if one exists: the
    /// one a crash between opening it and recording it left behind.
    fn find_open(&self, branch: &str) -> Result<Option<u64>, Error>;

    /// Opens a draft pull request of `branch` into `base`; its number.
    fn create_draft(&self, branch: &str, base: &str, title: &str, body: &str)
    -> Result<u64, Error>;

    /// Replaces a pull request's body, and its title when given.
    fn edit(&self, pr: &ItemRef, title: Option<&str>, body: &str) -> Result<(), Error>;

    /// Marks a draft pull request ready for review.
    fn ready(&self, pr: &ItemRef) -> Result<(), Error>;

    fn status(&self, pr: &ItemRef) -> Result<PrStatus, Error>;

    /// Waits for a pull request's checks; their exit code (0 green) and
    /// the output, which names the failed checks.
    fn checks_watch(&self, pr: &ItemRef) -> Result<(i32, String), Error>;

    /// The pull request's unresolved inline review threads.
    fn threads(&self, pr: &ItemRef) -> Result<Fetched, Error>;

    /// Posts `text` as a reply under the thread whose first comment is
    /// `comment`.
    fn reply(&self, pr: &ItemRef, comment: &str, text: &str) -> Result<(), Error>;

    /// Resolves the thread `thread`, whose first comment is `comment`.
    fn resolve(&self, pr: &ItemRef, thread: &str, comment: &str) -> Result<(), Error>;

    /// The log of a failed CI job: the latest failed run on `branch`
    /// unless the query names one.
    fn failed_log(&self, branch: &str, query: Query) -> Result<String, Error>;

    /// The error lines of [`CodeHost::failed_log`].
    fn failed(&self, branch: &str, query: Query) -> Result<Vec<String>, Error> {
        let log = self.failed_log(branch, query)?;
        Ok(error_lines(&log, query.context)
            .into_iter()
            .map(str::to_owned)
            .collect())
    }
}

/// The configured code host, running its programs in `root`. Credentials
/// are read on the first call that needs them, then kept.
#[must_use]
pub fn code_host(config: &Config, root: &Path) -> Box<dyn CodeHost> {
    match config.backend.ci {
        Ci::Github => Box::new(Gh::new(root).waiting(config.thresholds.ci_wait)),
        Ci::Bitbucket => Box::new(Bitbucket::new(root)),
    }
}

/// The comment id a host puts in an API path: digits only.
pub(crate) fn comment_id(comment: &str) -> Result<u64, Error> {
    comment
        .parse()
        .ok()
        .filter(|_| comment.chars().all(|c| c.is_ascii_digit()))
        .ok_or_else(|| Error::Ref(format!("comment '{comment}' is not a comment id")))
}
