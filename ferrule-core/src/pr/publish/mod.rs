//! What a run posts to its tracker and CI host: the tracking issue, slice,
//! split-off and follow-up issues, issue comments, the draft PR and its
//! body. Every body is rendered in memory, stripped of attribution lines
//! and posted; the number posted to is recorded on the parent card. An
//! open issue already titled exactly as the one to file is adopted, never
//! filed twice, and an open PR of the run's branch is adopted, never
//! opened twice, so a run resumed after a crash cannot duplicate one. Each
//! comment target and follow-up done is logged (`posted=`), so a rerun
//! after a partial failure does only what is missing.

use std::path::{Path, PathBuf};

use rusqlite::Connection;

use crate::body::{self, Form};
use crate::card::board::{self, CardId};
use crate::card::typed::{Name, PrState, Typed};
use crate::config::Config;
use crate::hook::attribution;
use crate::project::Layout;
use crate::refusal::{Exit, Refusal};
use crate::remote::{CodeHost, IssueTracker, ItemRef, PrStatus};
use crate::store::{Board, snapshot};
use crate::{git, remote, spec};

mod gate4;
mod open;
mod post;
mod route;

pub use gate4::PublishRefused;

#[derive(Debug)]
pub enum Error {
    Body(body::Error),
    /// An artifact the post needs is absent.
    Missing(PathBuf),
    /// The parent card has no `issue` to post to or cite.
    NoIssue,
    /// The parent card has no `branch` to open a PR from.
    NoBranch,
    /// No card for the run.
    Card,
    /// The run has no PR recorded.
    NoPr,
    /// A stage card the board would not move back: its number and why.
    Moved(u32, String),
    Remote(remote::Error),
    Sql(rusqlite::Error),
}

impl Error {
    /// The `<verb>=refused` line of an error the command answers with a
    /// refusal, among them an operation the configured host does not
    /// implement; `None` for the others (a missing artifact, the host's
    /// own error, the state database).
    #[must_use]
    pub fn refusal(&self, verb: &'static str) -> Option<Refusal> {
        match self {
            Error::Body(b) => b.refusal(verb),
            Error::NoIssue => Some(
                Refusal::new(verb, "no-issue")
                    .hint("file the tracking issue first: ferrule issue post <slug>"),
            ),
            Error::NoBranch => Some(
                Refusal::new(verb, "no-branch")
                    .hint("approve gate2 first: ferrule gate approve <slug> gate2"),
            ),
            Error::NoPr => Some(
                Refusal::new(verb, "no-pr")
                    .hint("no PR is recorded for this run; open it with `ferrule pr open <slug>`"),
            ),
            Error::Card => Some(Refusal::card(verb)),
            Error::Moved(n, why) => Some(
                Refusal::new(verb, "move")
                    .field("stage", format!("s{n}"))
                    .hint(format!("stage s{n} cannot move back to inprogress: {why}")),
            ),
            Error::Remote(remote::Error::Unsupported { host, op }) => Some(
                Refusal::new(verb, "unsupported")
                    .field("host", host)
                    .hint(format!(
                        "{op} is not implemented for {host} yet; do it on {host} by hand"
                    )),
            ),
            Error::Missing(_) | Error::Remote(_) | Error::Sql(_) => None,
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Body(e) => write!(f, "{e}"),
            Error::Missing(p) => write!(f, "{} is missing", p.display()),
            Error::NoIssue => f.write_str("the run's card has no issue"),
            Error::NoBranch => f.write_str("the run's card has no branch"),
            Error::Card => f.write_str("the run has no card"),
            Error::NoPr => f.write_str("the run has no PR recorded"),
            Error::Moved(n, why) => write!(f, "stage s{n} cannot move back: {why}"),
            Error::Remote(e) => write!(f, "{e}"),
            Error::Sql(e) => write!(f, "state database: {e}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Body(e) => Some(e),
            Error::Remote(e) => Some(e),
            Error::Sql(e) => Some(e),
            Error::Missing(_)
            | Error::NoIssue
            | Error::NoBranch
            | Error::Card
            | Error::NoPr
            | Error::Moved(..) => None,
        }
    }
}

impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Self {
        Error::Sql(e)
    }
}

impl From<remote::Error> for Error {
    fn from(e: remote::Error) -> Self {
        Error::Remote(e)
    }
}

impl From<body::Error> for Error {
    fn from(e: body::Error) -> Self {
        Error::Body(e)
    }
}

/// What `issue post` posts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// The run's tracking issue; an epic's is edited again to link its
    /// slices.
    Run,
    /// Slice n's own issue, recorded at position n of the parent's
    /// `slices:`.
    Slice(u32),
    /// split-issue.md, appended to the parent's `split:`.
    Split,
    /// follow-up-k.md.
    FollowUp(u32),
    /// issue-comment.md on the tracking issue (and a slice's epic).
    Comment,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Created,
    Edited,
    /// An open issue with the same title, now the run's.
    Adopted,
    Commented,
    /// The card already names the issue; nothing posted.
    Recorded,
}

impl Action {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Action::Created => "created",
            Action::Edited => "edited",
            Action::Adopted => "adopted",
            Action::Commented => "commented",
            Action::Recorded => "recorded",
        }
    }
}

/// `# <title>` then the body, as the body renderers and the agents write
/// them: the title and the body without its leading blank lines and
/// attribution lines.
#[must_use]
pub fn split_title(text: &str) -> (String, String) {
    let (first, rest) = text.split_once('\n').unwrap_or((text, ""));
    let title = first.strip_prefix("# ").unwrap_or(first).trim().to_owned();
    let body = rest.trim_start_matches('\n');
    let body = attribution::strip(body).unwrap_or_else(|| body.to_owned());
    (title, body)
}

/// `[71, 72]` as numbers.
#[must_use]
pub fn numbers(list: Option<&str>) -> Vec<u64> {
    list.unwrap_or("")
        .trim_matches(|c| c == '[' || c == ']')
        .split(',')
        .filter_map(|n| n.trim().trim_start_matches('#').parse().ok())
        .collect()
}

/// A `posted=` event's issue, sum and k.
type Posted = (Option<ItemRef>, Option<String>, Option<u32>);

/// The run's parent card, its artifacts and its hosts.
pub struct Run<'a> {
    pub(crate) conn: &'a mut Connection,
    pub layout: &'a Layout,
    pub config: &'a Config,
    /// Where `gh` runs: the repository's checkout.
    pub root: &'a Path,
    pub slug: &'a str,
    pub now: &'a str,
    /// The configured code host, the PR's.
    pub host: Box<dyn CodeHost>,
    /// The configured issue tracker, the issues'.
    pub tracker: Box<dyn IssueTracker>,
}

impl<'a> Run<'a> {
    /// The publish run for `slug` over the board `db`, on the configured
    /// code host and issue tracker.
    pub fn new(
        db: &'a mut Board,
        layout: &'a Layout,
        config: &'a Config,
        root: &'a Path,
        slug: &'a str,
        now: &'a str,
    ) -> Self {
        Run {
            conn: db.conn_mut(),
            layout,
            config,
            root,
            slug,
            now,
            host: crate::remote::code_host(config, root),
            tracker: crate::remote::issue_tracker(config, root),
        }
    }
}

impl Run<'_> {
    /// The run's worktree.
    pub(crate) fn worktree(&self) -> rusqlite::Result<PathBuf> {
        board::worktree(self.conn, self.layout, self.slug, self.slug)
    }

    pub(crate) fn field(&self, key: &str) -> rusqlite::Result<Option<String>> {
        Ok(board::field(self.conn, self.slug, key)?.filter(|v| !v.is_empty()))
    }

    pub(crate) fn set(&mut self, key: &str, value: &str) -> Result<(), Error> {
        board::set(self.conn, self.slug, key, value).map_err(|e| match e {
            board::CardError::Sql(e) => Error::Sql(e),
            _ => Error::Card,
        })
    }

    pub(crate) fn log(&mut self, event: &str, args: &[String]) -> Result<(), Error> {
        match board::log(self.conn, self.slug, event, args, self.now) {
            Ok(()) => Ok(()),
            Err(board::LogError::Sql(e)) => Err(Error::Sql(e)),
            Err(_) => Err(Error::Card),
        }
    }

    fn artifact(&self, name: &str) -> Result<String, Error> {
        let p = self.layout.run_dir(self.slug).join(name);
        std::fs::read_to_string(&p).map_err(|_| Error::Missing(p))
    }

    /// The `posted=<what>` events logged: `(issue, sum, k)` each.
    fn posted(&self, what: &str) -> rusqlite::Result<Vec<Posted>> {
        Ok(board::events(self.conn, self.slug, &[Name::Posted])?
            .iter()
            .filter_map(|e| match e.typed() {
                Typed::Posted {
                    what: w,
                    issue,
                    sum,
                    k,
                } if w == what => Some((
                    issue.and_then(|i| ItemRef::parse(i).ok()),
                    sum.map(str::to_owned),
                    k,
                )),
                _ => None,
            })
            .collect())
    }

    fn issue(&self) -> Result<ItemRef, Error> {
        remote::refs(self.field("issue")?.as_deref())
            .into_iter()
            .next()
            .ok_or(Error::NoIssue)
    }

    /// Adopts the user's open issue titled `title`, giving it `body`, else
    /// files a new one.
    fn adopt_or_create(&self, title: &str, body: &str) -> Result<(ItemRef, Action), Error> {
        if let Some(n) = self.tracker.find_open(title)? {
            self.tracker.edit(&n, title, body)?;
            return Ok((n, Action::Adopted));
        }
        Ok((self.tracker.create(title, body)?, Action::Created))
    }
}

impl Run<'_> {
    /// What git cannot answer about the run: the issue open and carrying a
    /// comment per kept spec change, the PR's state against the log, and
    /// its body against the board. One `differ` row each; a check the
    /// configured host cannot make ([`remote::Error::Unsupported`]) is
    /// skipped, not reported.
    pub fn reconcile_remote(&self) -> Result<Vec<String>, Error> {
        let mut differ = Vec::new();
        if let Some(issue) = remote::refs(self.field("issue")?.as_deref()).first() {
            let n = issue.cite();
            match self.tracker.view(issue) {
                Ok(item) => {
                    if !item.open {
                        differ.push(format!("issue {n} is not open"));
                    }
                    let kept = board::events(self.conn, self.slug, &[Name::SpecKeep])?.len();
                    if item.comments.len() < kept {
                        differ.push(format!(
                            "issue {n} has {} comments for {kept} kept spec changes",
                            item.comments.len()
                        ));
                    }
                }
                Err(remote::Error::Unsupported { .. }) => {}
                Err(e) => differ.push(format!("issue {n} unreadable: {e}")),
            }
        }
        if let Some(&n) = numbers(self.field("pr")?.as_deref()).first() {
            let ready = board::events(self.conn, self.slug, &[Name::Pr])?
                .iter()
                .any(|e| e.typed() == Typed::Pr(PrState::Ready));
            match self.host.status(&ItemRef::Number(n)) {
                Ok(PrStatus {
                    state,
                    draft,
                    body: current,
                }) => {
                    if state != remote::PrState::Open {
                        differ.push(format!("pr #{n} is {}", state.as_str()));
                    } else if ready && draft {
                        differ.push(format!("pr #{n} is a draft but pr=ready is logged"));
                    } else if !ready
                        && let Ok(rendered) = snapshot(self.conn, |r| {
                            body::pr(
                                r,
                                self.layout,
                                self.slug,
                                Form::Draft,
                                &self.config.main_branch,
                            )
                        })
                        && split_title(&rendered).1.trim_end() != current.trim_end()
                    {
                        differ.push(format!("pr #{n} body behind the board"));
                    }
                }
                Err(remote::Error::Unsupported { .. }) => {}
                Err(e) => differ.push(format!("pr #{n} unreadable: {e}")),
            }
        }
        Ok(differ)
    }
}

/// The `k` of every follow-up-k.md in the run's directory, sorted.
#[must_use]
pub fn follow_up_numbers(dir: &Path) -> Vec<u32> {
    crate::artifacts::follow_ups(dir)
        .into_iter()
        .map(|(k, _)| k)
        .collect()
}

/// What re-rendering the draft PR body did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Sync {
    Pushed,
    Unchanged,
    /// The body names a stage or an agent: the token; the PR keeps its
    /// previous body.
    Refused(String),
}

impl std::fmt::Display for Sync {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Sync::Pushed => f.write_str("pr-body=pushed"),
            Sync::Unchanged => f.write_str("pr-body=unchanged"),
            Sync::Refused(t) => write!(
                f,
                "{}",
                Refusal::new("pr-body", "stage-ref")
                    .field("stage-ref", t)
                    .hint("reword the text naming it; the PR keeps its previous body")
            ),
        }
    }
}

/// The run a card belongs to.
#[must_use]
pub fn run_of(card: &str) -> &str {
    CardId::parse(card).run
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pr::feedback::Thread;
    use crate::remote::fake::{FakeHost, FakeTracker};

    /// A board holding the parent card `r` with `fields`, a run directory
    /// with issue.md, and a fake host shared with the caller.
    fn setup(fields: &[(&str, &str)]) -> (tempfile::TempDir, Board, Layout, FakeHost) {
        let dir = tempfile::tempdir().unwrap();
        let layout = Layout::new(dir.path().to_owned());
        std::fs::create_dir_all(layout.run_dir("r")).unwrap();
        std::fs::write(
            layout.run_dir("r").join("issue.md"),
            "# Retry clients\n\n## Goal\nReconnect.\n",
        )
        .unwrap();
        let mut db = Board::memory().unwrap();
        let fields: Vec<(String, String)> = fields
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        board::create(
            db.conn_mut(),
            "r",
            board::State::InProgress,
            &fields,
            "2026-01-02T03:04",
        )
        .unwrap();
        (dir, db, layout, FakeHost::default())
    }

    fn run_on<'a>(
        db: &'a mut Board,
        layout: &'a Layout,
        config: &'a Config,
        host: &FakeHost,
    ) -> Run<'a> {
        let mut run = Run::new(db, layout, config, &layout.root, "r", "2026-01-02T03:04");
        run.host = Box::new(host.clone());
        run.tracker = Box::new(FakeTracker::default());
        run
    }

    #[test]
    fn ut_gate2_issue_adopts_the_open_issue_of_the_same_title() {
        let (_dir, mut db, layout, host) = setup(&[]);
        let tracker = FakeTracker::default();
        tracker.0.borrow_mut().open =
            vec![(30, "Retry client".into()), (31, "Retry clients".into())];
        let config = Config::default();
        let mut run = run_on(&mut db, &layout, &config, &host);
        run.tracker = Box::new(tracker.clone());
        assert_eq!(
            run.file(Target::Run, None).unwrap(),
            (ItemRef::Number(31), Action::Adopted)
        );
        assert_eq!(
            run.file(Target::Run, None).unwrap(),
            (ItemRef::Number(31), Action::Recorded)
        );
        assert_eq!(
            tracker.calls(),
            ["find_open Retry clients", "edit 31 Retry clients"],
            "adopted by its exact title and given the run's body, never filed"
        );
        assert_eq!(
            board::field(db.conn_mut(), "r", "issue")
                .unwrap()
                .as_deref(),
            Some("31")
        );
    }

    #[test]
    fn ut_gate2_issue_files_once_when_none_is_open() {
        let (_dir, mut db, layout, host) = setup(&[]);
        let tracker = FakeTracker::default();
        tracker.0.borrow_mut().next = 8;
        let config = Config::default();
        let mut run = run_on(&mut db, &layout, &config, &host);
        run.tracker = Box::new(tracker.clone());
        assert_eq!(
            run.file(Target::Run, None).unwrap(),
            (ItemRef::Number(8), Action::Created)
        );
        assert_eq!(
            run.file(Target::Run, None).unwrap(),
            (ItemRef::Number(8), Action::Recorded)
        );
        assert_eq!(
            tracker.calls(),
            ["find_open Retry clients", "create Retry clients"]
        );
    }

    #[test]
    fn ut_issue_comment_targets_the_issue_and_the_epic_once() {
        let (_dir, mut db, layout, host) =
            setup(&[("issue", "12"), ("kind", "slice"), ("epic", "#9")]);
        std::fs::write(layout.run_dir("r").join("issue-comment.md"), "Kept R3.\n").unwrap();
        let tracker = FakeTracker::default();
        let config = Config::default();
        let mut run = run_on(&mut db, &layout, &config, &host);
        run.tracker = Box::new(tracker.clone());
        assert_eq!(
            run.file(Target::Comment, None).unwrap(),
            (ItemRef::Number(12), Action::Commented)
        );
        assert_eq!(
            run.file(Target::Comment, None).unwrap(),
            (ItemRef::Number(12), Action::Recorded)
        );
        assert_eq!(
            tracker.calls(),
            ["view 12", "comment 12", "view 9", "comment 9"]
        );
    }

    #[test]
    fn ut_issue_comment_a_crash_left_unlogged_is_not_posted_again() {
        let (_dir, mut db, layout, host) = setup(&[("issue", "12")]);
        std::fs::write(layout.run_dir("r").join("issue-comment.md"), "Kept R3.\n").unwrap();
        let tracker = FakeTracker::default();
        let config = Config::default();
        let mut run = run_on(&mut db, &layout, &config, &host);
        run.tracker = Box::new(tracker.clone());
        run.file(Target::Comment, None).unwrap();
        let posted = tracker.0.borrow().comments.clone();
        assert!(
            posted[0].1.contains("<!-- ferrule:comment sum="),
            "{posted:?}"
        );
        // The posted= line is lost: the marker on the issue answers instead.
        let deleted = db
            .conn_mut()
            .execute("DELETE FROM events WHERE name LIKE 'posted%'", [])
            .unwrap();
        assert_eq!(deleted, 1);
        let mut run = run_on(&mut db, &layout, &config, &host);
        run.tracker = Box::new(tracker.clone());
        run.file(Target::Comment, None).unwrap();
        assert_eq!(tracker.0.borrow().comments.len(), 1);
    }

    #[test]
    fn ut_unsupported_tracker_write_is_a_refusal() {
        let (_dir, mut db, layout, host) = setup(&[]);
        let tracker = FakeTracker::default();
        tracker.0.borrow_mut().read_only = true;
        let config = Config::default();
        let mut run = run_on(&mut db, &layout, &config, &host);
        run.tracker = Box::new(tracker.clone());
        let e = run.file(Target::Run, None).unwrap_err();
        assert_eq!(
            e.refusal("post").unwrap().to_string(),
            "post=refused reason=unsupported host=Fake \
             hint=issue search is not implemented for Fake yet; do it on Fake by hand"
        );
        assert_eq!(board::field(db.conn_mut(), "r", "issue").unwrap(), None);
    }

    #[test]
    fn ut_publish_refuses_with_unresolved_threads() {
        let (_dir, mut db, layout, host) = setup(&[("gate4", "2026-01-02"), ("pr", "5")]);
        host.0.borrow_mut().threads = vec![Thread {
            id: "T1".into(),
            comment: "11".into(),
            path: "a.rs:3".into(),
            comments: vec![("al".into(), "t".into(), "fix".into())],
        }];
        let config = Config::default();
        let got = run_on(&mut db, &layout, &config, &host)
            .publish(None)
            .unwrap();
        assert_eq!(got, Err(PublishRefused::Threads(1)));
        assert_eq!(host.calls(), ["threads 5"]);
    }

    #[test]
    fn ut_open_pr_adopts_the_branch_open_pr() {
        let (_dir, mut db, layout, host) = setup(&[("branch", "feat/r")]);
        {
            let mut s = host.0.borrow_mut();
            s.open = Some(41);
            "open".clone_into(&mut s.state);
            s.draft = true;
        }
        let config = Config::default();
        let mut run = run_on(&mut db, &layout, &config, &host);
        assert_eq!(run.open_pr().unwrap(), (41, true));
        assert_eq!(run.open_pr().unwrap(), (41, false));
        assert_eq!(
            host.calls(),
            ["find_open feat/r", "status 41", "edit 41"],
            "adopted, its body synced, never opened"
        );
        assert!(host.0.borrow().body.starts_with("Draft"));
        assert_eq!(
            board::field(db.conn_mut(), "r", "pr").unwrap().as_deref(),
            Some("41")
        );
    }

    #[test]
    fn ut_open_pr_opens_one_when_none_is_open() {
        let (_dir, mut db, layout, host) = setup(&[("branch", "feat/r")]);
        host.0.borrow_mut().next = 7;
        let config = Config::default();
        let mut run = run_on(&mut db, &layout, &config, &host);
        assert_eq!(run.open_pr().unwrap(), (7, true));
        assert_eq!(
            host.calls(),
            [
                "find_open feat/r".to_owned(),
                format!("create_draft feat/r {}", config.main_branch)
            ]
        );
    }

    #[test]
    fn ut_title_and_body() {
        let (t, b) = split_title("# Retry clients\n\n## Goal\nx\n");
        assert_eq!(t, "Retry clients");
        assert_eq!(b, "## Goal\nx\n");
    }

    #[test]
    fn ut_numbers() {
        assert_eq!(numbers(Some("[71, #72]")), [71, 72]);
        assert!(numbers(None).is_empty());
    }
}
