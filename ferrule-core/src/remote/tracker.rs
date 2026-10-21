//! The issue tracker: viewing, filing, editing, commenting on and searching
//! issues. Built once from the configuration; an operation a tracker does
//! not implement answers [`Error::Unsupported`].

use std::path::Path;

use serde::Deserialize;

use super::github::Gh;
use super::jira::Jira;
use super::{Error, Item, ItemRef};
use crate::config::{Config, Tracker};

/// The most hits [`IssueTracker::find_open`] reads.
pub const FIND_LIMIT: usize = 100;

/// The search [`IssueTracker::find_open`] sends: the title as one quoted
/// phrase (its own double quotes dropped), so words like `-v` or
/// `is:closed` in it are matched, never read as qualifiers.
#[must_use]
pub fn title_query(title: &str) -> String {
    format!("\"{}\" in:title", title.replace('"', "").trim())
}

/// One hit of [`IssueTracker::search`].
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Found {
    pub number: u64,
    pub title: String,
    pub state: String,
}

/// The issue operations of a tracker.
pub trait IssueTracker {
    /// The tracker's name, as an [`Error::Unsupported`] names it.
    fn name(&self) -> &'static str;

    /// An issue with its comments: a GitHub number or URL, or a Jira key.
    fn view(&self, issue: &ItemRef) -> Result<Item, Error>;

    /// Files an issue; its number or key.
    fn create(&self, title: &str, body: &str) -> Result<ItemRef, Error>;

    /// Replaces an issue's title and body.
    fn edit(&self, issue: &ItemRef, title: &str, body: &str) -> Result<(), Error>;

    fn comment(&self, issue: &ItemRef, body: &str) -> Result<(), Error>;

    /// The open issue titled exactly `title`, if one exists among the
    /// first [`FIND_LIMIT`] hits of its quoted title.
    fn find_open(&self, title: &str) -> Result<Option<ItemRef>, Error>;

    /// Issues whose title or body mention every keyword (one with a space
    /// as a phrase), in `state` (`open`, `closed` or `all`); at most
    /// `limit` of them, newest first.
    fn search(&self, keywords: &[String], state: &str, limit: usize) -> Result<Vec<Found>, Error>;
}

/// The configured issue tracker, running its programs in `root`.
/// Credentials are read on the first call that needs them, then kept.
#[must_use]
pub fn issue_tracker(config: &Config, root: &Path) -> Box<dyn IssueTracker> {
    match config.backend.tracker {
        Tracker::Github => Box::new(Gh::new(root)),
        Tracker::Jira => Box::new(Jira::new(root)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ut_title_query_quotes_the_title() {
        assert_eq!(
            title_query("Add -v and is:closed \"modes\""),
            "\"Add -v and is:closed modes\" in:title"
        );
    }
}
