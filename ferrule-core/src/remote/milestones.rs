//! Milestones and issue relationships: what a tracker with native epics
//! offers beyond [`IssueTracker`](super::IssueTracker).

use super::{Error, ItemRef, SearchState};

/// A milestone with its description.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Milestone {
    pub number: u64,
    pub title: String,
    pub description: String,
    pub open: bool,
}

/// A blocker of an issue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Blocker {
    /// `#7`, or `owner/repo#7` in another repository.
    pub cite: String,
    pub reference: ItemRef,
    pub open: bool,
}

/// An issue's milestone, labels and relationships.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IssueLinks {
    pub open: bool,
    pub milestone: Option<Milestone>,
    pub labels: Vec<String>,
    pub parent: Option<ItemRef>,
    pub blocked_by: Vec<Blocker>,
    pub relates_to: Vec<ItemRef>,
    /// Open blockers GitHub counts but does not show.
    pub unreadable: u32,
}

/// An issue to file with its milestone and labels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewIssue<'a> {
    pub title: &'a str,
    pub body: &'a str,
    pub milestone: Option<&'a str>,
    pub labels: &'a [String],
}

/// The milestone and relationship operations of a tracker.
pub trait Milestones {
    /// The login running the tool.
    ///
    /// # Errors
    ///
    /// The host did not answer.
    fn login(&self) -> Result<String, Error>;

    /// # Errors
    ///
    /// The host did not answer.
    fn list(&self, state: SearchState) -> Result<Vec<Milestone>, Error>;

    /// # Errors
    ///
    /// The host did not answer, or has no such milestone.
    fn get(&self, number: u64) -> Result<Milestone, Error>;

    /// # Errors
    ///
    /// The host did not answer, or rejected the description.
    fn create(&self, title: &str, description: &str) -> Result<Milestone, Error>;

    /// # Errors
    ///
    /// The host did not answer, or rejected the description.
    fn set_description(&self, number: u64, description: &str) -> Result<(), Error>;

    /// Numbers of every issue in the milestone, ascending.
    ///
    /// # Errors
    ///
    /// The host did not answer.
    fn issues_in(&self, number: u64) -> Result<Vec<u64>, Error>;

    /// # Errors
    ///
    /// The host did not answer.
    fn find_open_in(&self, milestone: u64, title: &str) -> Result<Option<ItemRef>, Error>;

    /// # Errors
    ///
    /// The host did not answer.
    fn links(&self, issue: &ItemRef) -> Result<IssueLinks, Error>;

    /// # Errors
    ///
    /// The host did not answer.
    fn create_issue(&self, new: &NewIssue<'_>) -> Result<ItemRef, Error>;

    /// # Errors
    ///
    /// The host did not answer.
    fn set_milestone(&self, issue: &ItemRef, title: &str) -> Result<(), Error>;

    /// Creates whichever of `labels` the repository lacks.
    ///
    /// # Errors
    ///
    /// The host did not answer.
    fn ensure_labels(&self, labels: &[String]) -> Result<(), Error>;

    /// # Errors
    ///
    /// The host did not answer.
    fn add_labels(&self, issue: &ItemRef, labels: &[String]) -> Result<(), Error>;

    /// # Errors
    ///
    /// The host did not answer.
    fn add_blocked_by(&self, issue: &ItemRef, blocker: &ItemRef) -> Result<(), Error>;

    /// # Errors
    ///
    /// The host did not answer.
    fn set_parent(&self, issue: &ItemRef, parent: &ItemRef) -> Result<(), Error>;

    /// # Errors
    ///
    /// The host did not answer.
    fn add_relates_to(&self, issue: &ItemRef, other: &ItemRef) -> Result<(), Error>;

    /// # Errors
    ///
    /// The host did not answer.
    fn close(&self, issue: &ItemRef, comment: &str) -> Result<(), Error>;
}
