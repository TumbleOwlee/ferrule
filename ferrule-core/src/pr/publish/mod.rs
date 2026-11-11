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
    /// The milestone of the epic's title is closed: its number.
    MilestoneClosed(u64),
    /// A milestone text an edit expected is no longer there: the old text.
    MilestoneStale(String),
    /// A slice's `Depends on` names something that is no slice number.
    BadDependency(String),
    Remote(remote::Error),
    Sql(rusqlite::Error),
}

impl Error {
    /// The `<verb>=refused` line of an error the command answers with a
    /// refusal, a host failure among them ([`remote::Error::refusal`]);
    /// `None` for a body the command prints itself and the state database.
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
            Error::MilestoneClosed(n) => Some(
                Refusal::new(verb, "milestone-closed")
                    .field("milestone", n)
                    .hint(format!(
                        "milestone #{n} is closed; reopen it on GitHub, or retitle the draft or its Milestone: line, and rerun"
                    )),
            ),
            Error::MilestoneStale(_) => Some(Refusal::new(verb, "milestone-stale").hint(
                "another run changed that text; redraft milestone-change.md against `ferrule issue milestones --view <n>` and rerun",
            )),
            Error::BadDependency(d) => Some(
                Refusal::new(verb, "bad-dependency")
                    .field("value", d)
                    .hint("write `Depends on:` in slices.md as slice numbers or `none`, and rerun"),
            ),
            Error::Remote(e) => Some(e.refusal(verb)),
            Error::Missing(p) => Some(
                Refusal::new(verb, "missing")
                    .field(
                        "file",
                        p.file_name().unwrap_or(p.as_os_str()).to_string_lossy(),
                    )
                    .hint("write the run artifact first; `ferrule prompt render` names it"),
            ),
            Error::Sql(_) => None,
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
            Error::MilestoneClosed(n) => write!(f, "milestone #{n} is closed"),
            Error::MilestoneStale(t) => write!(f, "the milestone no longer holds: {t}"),
            Error::BadDependency(d) => write!(f, "`Depends on` names `{d}`, not a slice number"),
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
            | Error::MilestoneClosed(_)
            | Error::MilestoneStale(_)
            | Error::BadDependency(_)
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

/// What a comment post did: the issue commented and, on a slice run in a
/// milestone, the milestone and what happened to its description.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commented {
    pub issue: ItemRef,
    /// `Commented`, or `Recorded` when every target carries the text.
    pub action: Action,
    /// `Edited` the kept change was applied, `Recorded` its entry was there.
    pub milestone: Option<(u64, Action)>,
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
        tracker.0.borrow_mut().no_milestones = true;
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
        tracker.0.borrow_mut().no_milestones = true;
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
        let tracker = tracker_with(|s| s.no_milestones = true);
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

    /// RU-E-006 — a closed milestone refuses with its number and a way out.
    #[test]
    fn ut_milestone_refusals_name_their_fix() {
        let closed = Error::MilestoneClosed(7)
            .refusal("post")
            .unwrap()
            .to_string();
        assert!(
            closed.starts_with("post=refused reason=milestone-closed milestone=7 hint="),
            "{closed}"
        );
        assert!(closed.contains("milestone #7 is closed"), "{closed}");
        let stale = Error::MilestoneStale("x".into())
            .refusal("post")
            .unwrap()
            .to_string();
        assert!(
            stale.starts_with("post=refused reason=milestone-stale hint="),
            "{stale}"
        );
        assert!(stale.contains("ferrule issue milestones --view"), "{stale}");
    }

    const SLICES: &str = "## Slice 1: Core\nGoal: build core\nDepends on: none\nAreas: runs, ghost\n\n## Slice 2: Edge\nGoal: build edge\nDepends on: 1\n";

    /// An epic run: its artifacts, a spec area `runs`, and `fields`.
    fn epic_setup(fields: &[(&str, &str)]) -> (tempfile::TempDir, Board, Layout, FakeHost) {
        let mut all = vec![("kind", "epic")];
        all.extend_from_slice(fields);
        let (dir, db, layout, host) = setup(&all);
        let run = layout.run_dir("r");
        std::fs::write(run.join("slices.md"), SLICES).unwrap();
        std::fs::write(run.join("spec-diff.md"), "Area: runs — the runs\n").unwrap();
        std::fs::write(
            run.join("architecture.summary.md"),
            "Decide: use a pool\n\n| Component | Role |\n|---|---|\n| Pool | holds |\n",
        )
        .unwrap();
        std::fs::create_dir_all(layout.root.join("docs/specs/runs")).unwrap();
        (dir, db, layout, host)
    }

    fn milestone(number: u64, description: &str, open: bool) -> remote::Milestone {
        remote::Milestone {
            number,
            title: "Retry clients".to_owned(),
            description: description.to_owned(),
            open,
        }
    }

    fn tracker_with(setup: impl FnOnce(&mut remote::fake::Issues)) -> FakeTracker {
        let t = FakeTracker::default();
        setup(&mut t.0.borrow_mut());
        t
    }

    fn field(db: &mut Board, key: &str) -> Option<String> {
        board::field(db.conn_mut(), "r", key).unwrap()
    }

    /// RU-R-013 — an epic files one milestone and no issue.
    #[test]
    fn ut_epic_files_one_milestone_and_no_issue() {
        let (_dir, mut db, layout, host) = epic_setup(&[]);
        let tracker = tracker_with(|s| s.next = 3);
        let config = Config::default();
        let mut run = run_on(&mut db, &layout, &config, &host);
        run.tracker = Box::new(tracker.clone());
        assert!(run.files_milestone().unwrap());
        assert_eq!(run.file_epic(None).unwrap(), (3, Action::Created));
        assert_eq!(
            tracker.calls(),
            ["list all", "create_milestone Retry clients"]
        );
        assert_eq!(field(&mut db, "milestone").as_deref(), Some("3"));
        assert_eq!(field(&mut db, "issue"), None);
    }

    /// RU-R-013 — a run that is no epic, or a tracker without milestones, files none.
    #[test]
    fn ut_only_an_epic_on_a_milestone_tracker_files_a_milestone() {
        let (_dir, mut db, layout, host) = setup(&[]);
        let config = Config::default();
        let run = run_on(&mut db, &layout, &config, &host);
        assert!(!run.files_milestone().unwrap());
        let (_dir, mut db, layout, host) = epic_setup(&[]);
        let mut run = run_on(&mut db, &layout, &config, &host);
        run.tracker = Box::new(tracker_with(|s| s.no_milestones = true));
        assert!(!run.files_milestone().unwrap());
    }

    /// RU-R-014 — the description holds the brief and the skeleton, the slice list is not in it.
    #[test]
    fn ut_epic_description_holds_brief_and_skeleton() {
        let (_dir, mut db, layout, host) = epic_setup(&[]);
        let tracker = FakeTracker::default();
        let config = Config::default();
        let mut run = run_on(&mut db, &layout, &config, &host);
        run.tracker = Box::new(tracker.clone());
        run.file_epic(None).unwrap();
        let d = tracker.0.borrow().milestones[0].description.clone();
        assert!(d.contains("## Goal\nReconnect."), "{d}");
        assert!(d.contains("## Architecture\nuse a pool"), "{d}");
        assert!(!d.contains("## Slices"), "{d}");
        assert!(d.ends_with("## Changelog\n"), "{d}");
    }

    /// RU-R-019 — an epic on an existing issue closes it and never edits it.
    #[test]
    fn ut_epic_on_an_existing_issue_closes_it_unedited() {
        let (_dir, mut db, layout, host) = epic_setup(&[("named-issue", "12")]);
        let tracker = tracker_with(|s| {
            s.next = 3;
            s.open = vec![(12, "Old epic".into())];
            s.links.insert(
                12,
                remote::IssueLinks {
                    open: true,
                    ..Default::default()
                },
            );
        });
        let config = Config::default();
        let mut run = run_on(&mut db, &layout, &config, &host);
        run.tracker = Box::new(tracker.clone());
        assert_eq!(run.file_epic(None).unwrap(), (3, Action::Created));
        let calls = tracker.calls();
        assert!(calls.contains(&"close 12".to_owned()), "{calls:?}");
        assert!(!calls.iter().any(|c| c.starts_with("edit")), "{calls:?}");
        assert_eq!(field(&mut db, "issue").as_deref(), Some("12"));
        // The issue is closed now: a rerun closes nothing again.
        tracker.0.borrow_mut().links.get_mut(&12).unwrap().open = false;
        let mut run = run_on(&mut db, &layout, &config, &host);
        run.tracker = Box::new(tracker.clone());
        run.file_epic(None).unwrap();
        assert_eq!(
            tracker.calls().iter().filter(|c| *c == "close 12").count(),
            1
        );
    }

    /// RU-R-037 — a rerun adopts the open milestone of the title and rewrites its description.
    #[test]
    fn ut_rerun_adopts_the_open_milestone() {
        let (_dir, mut db, layout, host) = epic_setup(&[]);
        let tracker = tracker_with(|s| s.milestones = vec![milestone(4, "stale", true)]);
        let config = Config::default();
        let mut run = run_on(&mut db, &layout, &config, &host);
        run.tracker = Box::new(tracker.clone());
        assert_eq!(run.file_epic(None).unwrap(), (4, Action::Adopted));
        assert_eq!(field(&mut db, "milestone").as_deref(), Some("4"));
        assert!(tracker.calls().contains(&"set_description 4".to_owned()));
        assert!(
            !tracker
                .calls()
                .iter()
                .any(|c| c.starts_with("create_milestone"))
        );
        let mut run = run_on(&mut db, &layout, &config, &host);
        run.tracker = Box::new(tracker.clone());
        assert_eq!(run.file_epic(None).unwrap(), (4, Action::Recorded));
        assert_eq!(
            tracker
                .calls()
                .iter()
                .filter(|c| c.starts_with("set_description"))
                .count(),
            1,
            "an unchanged description is not written again"
        );
        tracker.0.borrow_mut().milestones[0].description = "edited by hand".into();
        let mut run = run_on(&mut db, &layout, &config, &host);
        run.tracker = Box::new(tracker.clone());
        assert_eq!(run.file_epic(None).unwrap(), (4, Action::Edited));
    }

    /// RU-R-037 — a rerun keeps the changelog entries the description already holds.
    #[test]
    fn ut_rerun_keeps_the_changelog() {
        let (_dir, mut db, layout, host) = epic_setup(&[("milestone", "4")]);
        let entry = crate::issue::changelog::Entry {
            at: "2026-01-02T03:04",
            author: "ann",
            decision: "keep",
            summary: "kept",
            sum: "abc",
        };
        let old = crate::issue::changelog::append("old brief\n", &entry);
        let tracker = tracker_with(|s| s.milestones = vec![milestone(4, &old, true)]);
        let config = Config::default();
        let mut run = run_on(&mut db, &layout, &config, &host);
        run.tracker = Box::new(tracker.clone());
        assert_eq!(run.file_epic(None).unwrap(), (4, Action::Edited));
        let d = tracker.0.borrow().milestones[0].description.clone();
        assert!(d.contains("ferrule:change sum=abc"), "{d}");
        assert!(d.contains("## Goal"), "{d}");
        assert!(!d.contains("old brief"), "{d}");
    }

    /// RU-E-006 — a closed milestone of the title, by card or by title, refuses.
    #[test]
    fn ut_a_closed_milestone_of_the_title_refuses() {
        let config = Config::default();
        for fields in [&[][..], &[("milestone", "4")]] {
            let (_dir, mut db, layout, host) = epic_setup(fields);
            let tracker = tracker_with(|s| s.milestones = vec![milestone(4, "x", false)]);
            let mut run = run_on(&mut db, &layout, &config, &host);
            run.tracker = Box::new(tracker.clone());
            assert!(matches!(
                run.file_epic(None),
                Err(Error::MilestoneClosed(4))
            ));
            let calls = tracker.calls();
            assert!(
                !calls
                    .iter()
                    .any(|c| c.starts_with("create") || c.starts_with("set_description")),
                "{calls:?}"
            );
        }
    }

    /// RU-E-008 — filing an epic closes the existing issue only, never a milestone.
    #[test]
    fn ut_epic_filing_never_closes_a_milestone() {
        let (_dir, mut db, layout, host) = epic_setup(&[]);
        let tracker = tracker_with(|s| s.milestones = vec![milestone(4, "x", true)]);
        let config = Config::default();
        for _ in 0..2 {
            let mut run = run_on(&mut db, &layout, &config, &host);
            run.tracker = Box::new(tracker.clone());
            run.file_epic(None).unwrap();
        }
        assert!(
            !tracker.calls().iter().any(|c| c.starts_with("close")),
            "{:?}",
            tracker.calls()
        );
        assert!(tracker.0.borrow().milestones[0].open);
    }

    /// An epic run inside milestone 4, on a tracker numbering from 20.
    fn slice_setup() -> (tempfile::TempDir, Board, Layout, FakeHost, FakeTracker) {
        let (dir, db, layout, host) = epic_setup(&[("milestone", "4")]);
        let tracker = tracker_with(|s| {
            s.next = 20;
            s.milestones = vec![milestone(4, "brief", true)];
        });
        (dir, db, layout, host, tracker)
    }

    fn file_slice(
        db: &mut Board,
        layout: &Layout,
        host: &FakeHost,
        tracker: &FakeTracker,
        n: u32,
    ) -> Result<(ItemRef, Action), Error> {
        let config = Config::default();
        let mut run = run_on(db, layout, &config, host);
        run.tracker = Box::new(tracker.clone());
        run.file(Target::Slice(n), None)
    }

    /// The writes among the calls after the first `from`.
    fn writes(tracker: &FakeTracker, from: usize) -> Vec<String> {
        tracker.calls()[from..]
            .iter()
            .filter(|c| {
                ["edit", "add_", "ensure_", "create", "set_", "close"]
                    .iter()
                    .any(|w| c.starts_with(w))
            })
            .cloned()
            .collect()
    }

    /// RU-R-016 — a slice is created inside the epic's milestone, labelled before it is filed.
    #[test]
    fn ut_slice_is_created_in_the_epic_milestone() {
        let (_dir, mut db, layout, host, tracker) = slice_setup();
        assert_eq!(
            file_slice(&mut db, &layout, &host, &tracker, 1).unwrap(),
            (ItemRef::Number(20), Action::Created)
        );
        let calls = tracker.calls();
        let at = |c: &str| calls.iter().position(|x| x == c).unwrap_or(usize::MAX);
        assert!(at("find_open_in 4 Core") < at("create_issue Core milestone=4 labels=area:runs"));
        assert!(
            at("ensure_labels area:runs") < at("create_issue Core milestone=4 labels=area:runs")
        );
        assert_eq!(field(&mut db, "slices").as_deref(), Some("[20]"));
        assert_eq!(
            file_slice(&mut db, &layout, &host, &tracker, 1).unwrap(),
            (ItemRef::Number(20), Action::Recorded)
        );
    }

    /// RU-R-016 — a slice run without the epic's milestone on its card refuses.
    #[test]
    fn ut_slice_without_a_milestone_refuses() {
        let (_dir, mut db, layout, host) = epic_setup(&[]);
        let tracker = FakeTracker::default();
        assert!(matches!(
            file_slice(&mut db, &layout, &host, &tracker, 1),
            Err(Error::NoIssue)
        ));
        assert!(tracker.calls().is_empty());
    }

    /// RU-R-023 — a slice is blocked by each slice it depends on; an unfiled one refuses first.
    #[test]
    fn ut_slice_gets_blocked_by_its_dependency() {
        let (_dir, mut db, layout, host, tracker) = slice_setup();
        let e = file_slice(&mut db, &layout, &host, &tracker, 2).unwrap_err();
        assert!(matches!(e, Error::Body(body::Error::NoSlice(1))), "{e}");
        assert!(writes(&tracker, 0).is_empty(), "{:?}", tracker.calls());
        file_slice(&mut db, &layout, &host, &tracker, 1).unwrap();
        file_slice(&mut db, &layout, &host, &tracker, 2).unwrap();
        assert!(tracker.calls().contains(&"add_blocked_by 21 20".to_owned()));
        assert_eq!(field(&mut db, "slices").as_deref(), Some("[20, 21]"));
    }

    /// A `Depends on` that is no slice number refuses, naming the value, and writes nothing.
    #[test]
    fn ut_slice_with_a_non_numeric_dependency_names_it() {
        let (_dir, mut db, layout, host, tracker) = slice_setup();
        std::fs::write(
            layout.run_dir("r").join("slices.md"),
            "## Slice 1: Core\nGoal: a\nDepends on: none\n\n## Slice 2: Edge\nGoal: b\nDepends on: core\n",
        )
        .unwrap();
        file_slice(&mut db, &layout, &host, &tracker, 1).unwrap();
        let before = tracker.calls().len();
        let e = file_slice(&mut db, &layout, &host, &tracker, 2).unwrap_err();
        assert!(writes(&tracker, before).is_empty(), "{:?}", tracker.calls());
        assert!(matches!(&e, Error::BadDependency(d) if d == "core"), "{e}");
        let line = e.refusal("issue").unwrap().to_string();
        assert!(line.contains("reason=bad-dependency"), "{line}");
        assert!(line.contains("value=core"), "{line}");
    }

    /// RU-R-024 — a slice's `Areas:` key labels it with each area the specs have; the brief's
    /// `Area:` line stands in without the key.
    #[test]
    fn ut_slice_carries_its_area_labels() {
        let (_dir, mut db, layout, host, tracker) = slice_setup();
        file_slice(&mut db, &layout, &host, &tracker, 1).unwrap();
        file_slice(&mut db, &layout, &host, &tracker, 2).unwrap();
        let calls = tracker.calls();
        assert!(
            calls.contains(&"create_issue Core milestone=4 labels=area:runs".to_owned()),
            "{calls:?}"
        );
        assert!(
            calls.contains(&"create_issue Edge milestone=4 labels=area:runs".to_owned()),
            "{calls:?}"
        );
        // No spec directory, no label, and no label call.
        std::fs::write(
            layout.run_dir("r").join("spec-diff.md"),
            "Area: ghost — x\n",
        )
        .unwrap();
        db.conn_mut()
            .execute("DELETE FROM card_fields WHERE key = 'slices'", [])
            .unwrap();
        tracker.0.borrow_mut().open.clear();
        let from = tracker.calls().len();
        file_slice(&mut db, &layout, &host, &tracker, 1).unwrap();
        file_slice(&mut db, &layout, &host, &tracker, 2).unwrap();
        let later = tracker.calls()[from..].to_vec();
        assert!(
            later.contains(&"create_issue Edge milestone=4 labels=".to_owned()),
            "{later:?}"
        );
        assert!(
            !later
                .iter()
                .any(|c| c.starts_with("ensure_labels area:ghost"))
        );
    }

    /// RU-R-030 — a rerun after a failure writes only the relationship and label still missing.
    #[test]
    fn ut_rerun_writes_only_missing_relationships() {
        let (_dir, mut db, layout, host, tracker) = slice_setup();
        file_slice(&mut db, &layout, &host, &tracker, 1).unwrap();
        file_slice(&mut db, &layout, &host, &tracker, 2).unwrap();
        let blocker = remote::Blocker {
            cite: "#20".into(),
            reference: ItemRef::Number(20),
            open: true,
        };
        let reset = |db: &mut Board| {
            board::set(db.conn_mut(), "r", "slices", "[20, -]").unwrap();
        };
        // Everything is there: the rerun writes nothing.
        reset(&mut db);
        tracker.0.borrow_mut().links.insert(
            21,
            remote::IssueLinks {
                open: true,
                labels: vec!["area:runs".into()],
                blocked_by: vec![blocker],
                ..Default::default()
            },
        );
        let from = tracker.calls().len();
        assert_eq!(
            file_slice(&mut db, &layout, &host, &tracker, 2).unwrap(),
            (ItemRef::Number(21), Action::Adopted)
        );
        assert!(
            writes(&tracker, from).is_empty(),
            "{:?}",
            writes(&tracker, from)
        );
        // The blocker is missing: only it is written.
        reset(&mut db);
        tracker
            .0
            .borrow_mut()
            .links
            .get_mut(&21)
            .unwrap()
            .blocked_by
            .clear();
        let from = tracker.calls().len();
        file_slice(&mut db, &layout, &host, &tracker, 2).unwrap();
        assert_eq!(writes(&tracker, from), ["add_blocked_by 21 20"]);
    }

    /// RU-R-030 — an adopted slice is edited only when its text differs, and gets only the
    /// labels it lacks.
    #[test]
    fn ut_adopted_slice_edits_and_labels_only_what_differs() {
        let (_dir, mut db, layout, host, tracker) = slice_setup();
        std::fs::create_dir_all(layout.root.join("docs/specs/card")).unwrap();
        std::fs::write(
            layout.run_dir("r").join("slices.md"),
            "## Slice 1: Core\nGoal: build core\nDepends on: none\nAreas: runs, card\n",
        )
        .unwrap();
        file_slice(&mut db, &layout, &host, &tracker, 1).unwrap();
        let reset = |db: &mut Board| board::set(db.conn_mut(), "r", "slices", "[-]").unwrap();
        reset(&mut db);
        tracker.0.borrow_mut().links.insert(
            20,
            remote::IssueLinks {
                open: true,
                labels: vec!["area:runs".into()],
                ..Default::default()
            },
        );
        let from = tracker.calls().len();
        file_slice(&mut db, &layout, &host, &tracker, 1).unwrap();
        assert_eq!(
            writes(&tracker, from),
            ["ensure_labels area:card", "add_labels 20 area:card"],
            "the same text is not edited, a present label not added"
        );
        reset(&mut db);
        tracker.0.borrow_mut().bodies.insert(20, "other\n".into());
        let from = tracker.calls().len();
        file_slice(&mut db, &layout, &host, &tracker, 1).unwrap();
        assert_eq!(writes(&tracker, from)[0], "edit 20 Core");
    }

    /// RU-E-012 — a slice adopts an open issue of its title only inside the milestone.
    #[test]
    fn ut_slice_adopts_only_inside_the_milestone() {
        let (_dir, mut db, layout, host, tracker) = slice_setup();
        tracker.0.borrow_mut().open = vec![(30, "Core".into())];
        assert_eq!(
            file_slice(&mut db, &layout, &host, &tracker, 1).unwrap(),
            (ItemRef::Number(20), Action::Created)
        );
        assert!(!tracker.calls().iter().any(|c| c.starts_with("edit 30")));
    }

    /// RU-R-030 — a tracking issue whose text is unchanged is not rewritten on a rerun.
    #[test]
    fn ut_tracking_issue_rerun_rewrites_nothing_unchanged() {
        let (_dir, mut db, layout, host) = setup(&[]);
        let tracker = tracker_with(|s| s.open = vec![(31, "Retry clients".into())]);
        let config = Config::default();
        for _ in 0..2 {
            let mut run = run_on(&mut db, &layout, &config, &host);
            run.tracker = Box::new(tracker.clone());
            assert_eq!(
                run.file(Target::Run, None).unwrap(),
                (ItemRef::Number(31), Action::Adopted)
            );
            db.conn_mut()
                .execute("DELETE FROM card_fields WHERE key = 'issue'", [])
                .unwrap();
        }
        let edits = tracker
            .calls()
            .iter()
            .filter(|c| c.starts_with("edit"))
            .count();
        assert_eq!(edits, 1, "{:?}", tracker.calls());
    }

    /// RU-R-024 — a tracking issue, created, adopted or reused, gets the label of each area the
    /// brief names, once.
    #[test]
    fn ut_tracking_issue_gets_its_area_label() {
        let config = Config::default();
        for (open, reuse, issue) in [
            (vec![], None, 8),
            (vec![(31, "Retry clients".to_owned())], None, 31),
            (vec![], Some(ItemRef::Number(40)), 40),
        ] {
            let (_dir, mut db, layout, host) = setup(&[]);
            std::fs::write(layout.run_dir("r").join("spec-diff.md"), "Area: runs — x\n").unwrap();
            std::fs::create_dir_all(layout.root.join("docs/specs/runs")).unwrap();
            let tracker = tracker_with(|s| {
                s.next = 8;
                s.open = open;
            });
            let mut run = run_on(&mut db, &layout, &config, &host);
            run.tracker = Box::new(tracker.clone());
            run.file(Target::Run, reuse).unwrap();
            let calls = tracker.calls();
            assert!(
                calls.contains(&"ensure_labels area:runs".to_owned()),
                "{calls:?}"
            );
            // A created issue carries its labels from the start; the others are labelled after.
            let created = issue == 8;
            assert_eq!(
                calls.contains(&format!("add_labels {issue} area:runs")),
                !created,
                "{calls:?}"
            );
            assert_eq!(
                calls.iter().filter(|c| c.starts_with("add_labels")).count(),
                usize::from(!created),
                "{calls:?}"
            );
        }
    }

    const SPLIT: &str = "# Split title\n\nBody of split.\n";
    const FOLLOW: &str = "# Follow title\n\nBody of follow.\n";

    /// A run on issue 12 with spec areas `runs` and `card`, a brief naming `runs`, a split-off
    /// and a follow-up draft.
    fn linked_setup(fields: &[(&str, &str)]) -> (tempfile::TempDir, Board, Layout, FakeHost) {
        let mut all = vec![("issue", "12")];
        all.extend_from_slice(fields);
        let (dir, db, layout, host) = setup(&all);
        let run = layout.run_dir("r");
        std::fs::write(run.join("spec-diff.md"), "Area: runs — the runs\n").unwrap();
        std::fs::write(run.join("split-issue.md"), SPLIT).unwrap();
        std::fs::write(run.join("follow-up-1.md"), FOLLOW).unwrap();
        for area in ["runs", "card"] {
            std::fs::create_dir_all(layout.root.join("docs/specs").join(area)).unwrap();
        }
        (dir, db, layout, host)
    }

    /// A tracker numbering from 8 with milestone 4 and `extra` applied.
    fn linked_tracker(extra: impl FnOnce(&mut remote::fake::Issues)) -> FakeTracker {
        tracker_with(|s| {
            s.next = 8;
            s.milestones = vec![milestone(4, "brief", true)];
            extra(s);
        })
    }

    fn file_target(
        db: &mut Board,
        layout: &Layout,
        host: &FakeHost,
        tracker: &FakeTracker,
        target: Target,
    ) -> Result<(ItemRef, Action), Error> {
        let config = Config::default();
        let mut run = run_on(db, layout, &config, host);
        run.tracker = Box::new(tracker.clone());
        run.file(target, None)
    }

    fn comment_on(
        db: &mut Board,
        layout: &Layout,
        host: &FakeHost,
        tracker: &FakeTracker,
    ) -> Result<Commented, Error> {
        let config = Config::default();
        let mut run = run_on(db, layout, &config, host);
        run.tracker = Box::new(tracker.clone());
        run.comment()
    }

    /// RU-R-025 — a split-off issue gets the source as its parent and carries no `Split from`
    /// line.
    #[test]
    fn ut_split_sets_parent_and_carries_no_line() {
        let (_dir, mut db, layout, host) = linked_setup(&[]);
        let tracker = linked_tracker(|_| {});
        assert_eq!(
            file_target(&mut db, &layout, &host, &tracker, Target::Split).unwrap(),
            (ItemRef::Number(8), Action::Created)
        );
        let calls = tracker.calls();
        assert!(calls.contains(&"set_parent 8 12".to_owned()), "{calls:?}");
        let body = tracker.0.borrow().bodies[&8].clone();
        assert!(!body.contains("Split from"), "{body}");
        assert!(body.contains("Body of split."), "{body}");
        assert_eq!(field(&mut db, "split").as_deref(), Some("[8]"));
    }

    /// RU-R-021, RU-R-026 — a split-off of a slice run is filed in the slice's milestone, which
    /// records the split in its changelog once.
    #[test]
    fn ut_split_of_a_slice_is_in_its_milestone() {
        let (_dir, mut db, layout, host) = linked_setup(&[("kind", "slice"), ("milestone", "4")]);
        let tracker = linked_tracker(|s| s.login = "ann".into());
        file_target(&mut db, &layout, &host, &tracker, Target::Split).unwrap();
        let calls = tracker.calls();
        assert!(
            calls.contains(&"find_open_in 4 Split title".to_owned()),
            "{calls:?}"
        );
        assert!(
            calls
                .iter()
                .any(|c| c.starts_with("create_issue Split title milestone=4")),
            "{calls:?}"
        );
        let d = tracker.0.borrow().milestones[0].description.clone();
        assert!(d.contains("@ann · Split off #8 — Split title"), "{d}");
        // A rerun records no second entry.
        board::set(db.conn_mut(), "r", "split", "").unwrap();
        file_target(&mut db, &layout, &host, &tracker, Target::Split).unwrap();
        let d = tracker.0.borrow().milestones[0].description.clone();
        assert_eq!(d.matches("ferrule:change").count(), 1, "{d}");
    }

    /// RU-R-030 — a split-off whose draft was filed is recorded with it: a rerun inside the
    /// host's list lag files no second issue, and a changed draft files its own.
    #[test]
    fn ut_split_rerun_inside_list_lag_files_once() {
        let (_dir, mut db, layout, host) = linked_setup(&[("kind", "slice"), ("milestone", "4")]);
        let tracker = linked_tracker(|s| s.login = "ann".into());
        let (n, action) = file_target(&mut db, &layout, &host, &tracker, Target::Split).unwrap();
        assert_eq!(action, Action::Created);
        // The host's list has not caught up: it answers nothing for the milestone.
        tracker.0.borrow_mut().open.clear();
        tracker.0.borrow_mut().issues_in.clear();
        let (again, action) =
            file_target(&mut db, &layout, &host, &tracker, Target::Split).unwrap();
        assert_eq!((again, action), (n, Action::Recorded));
        let creates = |t: &FakeTracker| {
            t.calls()
                .iter()
                .filter(|c| c.starts_with("create_issue"))
                .count()
        };
        assert_eq!(creates(&tracker), 1, "{:?}", tracker.calls());
        std::fs::write(
            layout.run_dir("r").join("split-issue.md"),
            "# Another split\n\nBody\n",
        )
        .unwrap();
        file_target(&mut db, &layout, &host, &tracker, Target::Split).unwrap();
        assert_eq!(creates(&tracker), 2, "{:?}", tracker.calls());
    }

    /// RU-R-030 — a split-off whose parent write failed is recorded when filed: a rerun inside
    /// the host's list lag files no second issue and writes the missing parent.
    #[test]
    fn ut_split_failed_midway_is_recorded_and_completed() {
        let (_dir, mut db, layout, host) = linked_setup(&[("kind", "slice"), ("milestone", "4")]);
        let tracker = linked_tracker(|s| {
            s.login = "ann".into();
            s.fail_on = Some("set_parent");
        });
        assert!(file_target(&mut db, &layout, &host, &tracker, Target::Split).is_err());
        tracker.0.borrow_mut().fail_on = None;
        tracker.0.borrow_mut().open.clear();
        tracker.0.borrow_mut().issues_in.clear();
        let (n, action) = file_target(&mut db, &layout, &host, &tracker, Target::Split).unwrap();
        assert_eq!((n, action), (ItemRef::Number(8), Action::Recorded));
        let calls = tracker.calls();
        let count = |p: &str| calls.iter().filter(|c| c.starts_with(p)).count();
        assert_eq!(count("create_issue"), 1, "{calls:?}");
        assert_eq!(count("set_parent 8 12"), 2, "{calls:?}");
        let d = tracker.0.borrow().milestones[0].description.clone();
        assert_eq!(d.matches("ferrule:change").count(), 1, "{d}");
    }

    /// RU-R-024, RU-R-030 — a tracking issue is created with its area labels in one call, so a
    /// label write cannot fail after the issue exists.
    #[test]
    fn ut_created_tracking_issue_carries_its_labels() {
        let (_dir, mut db, layout, host) = setup(&[]);
        std::fs::write(layout.run_dir("r").join("spec-diff.md"), "Area: runs — x\n").unwrap();
        std::fs::create_dir_all(layout.root.join("docs/specs/runs")).unwrap();
        let tracker = tracker_with(|s| s.next = 8);
        let config = Config::default();
        let mut run = run_on(&mut db, &layout, &config, &host);
        run.tracker = Box::new(tracker.clone());
        run.file(Target::Run, None).unwrap();
        let calls = tracker.calls();
        assert!(
            calls.contains(&"create_issue Retry clients milestone=- labels=area:runs".to_owned()),
            "{calls:?}"
        );
        assert!(
            !calls.iter().any(|c| c.starts_with("add_labels")),
            "{calls:?}"
        );
    }

    /// RU-E-017 — an adopted split-off keeps the parent it already has.
    #[test]
    fn ut_adopted_split_keeps_its_other_parent() {
        let (_dir, mut db, layout, host) = linked_setup(&[]);
        let tracker = linked_tracker(|s| {
            s.open = vec![(30, "Split title".into())];
            s.links.insert(
                30,
                remote::IssueLinks {
                    open: true,
                    parent: Some(ItemRef::Number(9)),
                    ..Default::default()
                },
            );
        });
        assert_eq!(
            file_target(&mut db, &layout, &host, &tracker, Target::Split).unwrap(),
            (ItemRef::Number(30), Action::Adopted)
        );
        assert!(!tracker.calls().iter().any(|c| c.starts_with("set_parent")));
    }

    /// RU-R-024 — a split-off carries the labels of the areas its draft names, else the brief's.
    #[test]
    fn ut_split_carries_its_area_labels() {
        let (_dir, mut db, layout, host) = linked_setup(&[]);
        let tracker = linked_tracker(|_| {});
        std::fs::write(
            layout.run_dir("r").join("split-issue.md"),
            "# Split title\n\nAreas: card, ghost\nBody of split.\n",
        )
        .unwrap();
        file_target(&mut db, &layout, &host, &tracker, Target::Split).unwrap();
        let calls = tracker.calls();
        assert!(
            calls.contains(&"create_issue Split title milestone=- labels=area:card".to_owned()),
            "{calls:?}"
        );
        let body = tracker.0.borrow().bodies[&8].clone();
        assert!(!body.contains("Areas:"), "{body}");
        board::set(db.conn_mut(), "r", "split", "").unwrap();
        std::fs::write(layout.run_dir("r").join("split-issue.md"), SPLIT).unwrap();
        tracker.0.borrow_mut().open.clear();
        file_target(&mut db, &layout, &host, &tracker, Target::Split).unwrap();
        assert!(
            tracker
                .calls()
                .contains(&"create_issue Split title milestone=- labels=area:runs".to_owned())
        );
    }

    /// RU-R-024 — an adopted split-off gets only the labels it lacks, and no text rewrite when
    /// it already holds the draft.
    #[test]
    fn ut_adopted_split_gets_only_missing_labels() {
        let (_dir, mut db, layout, host) = linked_setup(&[]);
        std::fs::write(
            layout.run_dir("r").join("split-issue.md"),
            "# Split title\n\nAreas: runs, card\nBody of split.\n",
        )
        .unwrap();
        let tracker = linked_tracker(|s| {
            s.open = vec![(30, "Split title".into())];
            s.bodies.insert(30, "Body of split.\n".into());
            s.links.insert(
                30,
                remote::IssueLinks {
                    open: true,
                    labels: vec!["area:runs".into()],
                    parent: Some(ItemRef::Number(12)),
                    ..Default::default()
                },
            );
        });
        file_target(&mut db, &layout, &host, &tracker, Target::Split).unwrap();
        assert_eq!(
            writes(&tracker, 0),
            ["ensure_labels area:card", "add_labels 30 area:card"]
        );
    }

    /// RU-R-030 — a follow-up recorded as filed still gets its `relates to` link when the host
    /// lacks it, and files no second issue.
    #[test]
    fn ut_recorded_follow_up_completes_its_link() {
        let (_dir, mut db, layout, host) = linked_setup(&[]);
        let tracker = linked_tracker(|_| {});
        file_target(&mut db, &layout, &host, &tracker, Target::FollowUp(1)).unwrap();
        // The fake keeps no links: the host lacks the one just added.
        let again = file_target(&mut db, &layout, &host, &tracker, Target::FollowUp(1)).unwrap();
        assert_eq!(again, (ItemRef::Number(8), Action::Recorded));
        let calls = tracker.calls();
        let n = |p: &str| calls.iter().filter(|c| c.starts_with(p)).count();
        assert_eq!(n("add_relates_to 8 12"), 2, "{calls:?}");
        assert_eq!(n("create_issue"), 1, "{calls:?}");
    }

    /// RU-R-027 — a follow-up gets a `relates to` link to the run's issue and no `Found in`
    /// line; a rerun adds the link only when it is missing.
    #[test]
    fn ut_follow_up_relates_and_carries_no_line() {
        let (_dir, mut db, layout, host) = linked_setup(&[]);
        let tracker = linked_tracker(|_| {});
        assert_eq!(
            file_target(&mut db, &layout, &host, &tracker, Target::FollowUp(1)).unwrap(),
            (ItemRef::Number(8), Action::Created)
        );
        assert!(tracker.calls().contains(&"add_relates_to 8 12".to_owned()));
        let body = tracker.0.borrow().bodies[&8].clone();
        assert!(!body.contains("Found in"), "{body}");
        assert!(body.contains("Body of follow."), "{body}");
        db.conn_mut()
            .execute("DELETE FROM events WHERE name LIKE 'posted%'", [])
            .unwrap();
        tracker.0.borrow_mut().links.insert(
            8,
            remote::IssueLinks {
                open: true,
                relates_to: vec![ItemRef::Number(12)],
                labels: vec!["area:runs".into()],
                ..Default::default()
            },
        );
        let from = tracker.calls().len();
        file_target(&mut db, &layout, &host, &tracker, Target::FollowUp(1)).unwrap();
        assert!(
            writes(&tracker, from).is_empty(),
            "{:?}",
            writes(&tracker, from)
        );
    }

    /// RU-R-028 — a follow-up of a slice run lands in the milestone its draft names.
    #[test]
    fn ut_follow_up_lands_in_the_named_milestone() {
        let (_dir, mut db, layout, host) = linked_setup(&[("kind", "slice"), ("milestone", "4")]);
        std::fs::write(
            layout.run_dir("r").join("follow-up-1.md"),
            "# Follow title\n\nMilestone: Other\nBody of follow.\n",
        )
        .unwrap();
        let mut other = milestone(5, "x", true);
        "Other".clone_into(&mut other.title);
        let tracker = linked_tracker(|s| s.milestones.push(other));
        file_target(&mut db, &layout, &host, &tracker, Target::FollowUp(1)).unwrap();
        let calls = tracker.calls();
        assert!(
            calls.contains(&"find_open_in 5 Follow title".to_owned()),
            "{calls:?}"
        );
        assert!(
            calls
                .iter()
                .any(|c| c.starts_with("create_issue Follow title milestone=5")),
            "{calls:?}"
        );
        let body = tracker.0.borrow().bodies[&8].clone();
        assert!(!body.contains("Milestone:"), "{body}");
    }

    /// RU-E-015 — a follow-up naming a closed milestone refuses and files nothing.
    #[test]
    fn ut_follow_up_naming_a_closed_milestone_refuses() {
        let (_dir, mut db, layout, host) = linked_setup(&[("kind", "slice"), ("milestone", "4")]);
        std::fs::write(
            layout.run_dir("r").join("follow-up-1.md"),
            "# Follow title\n\nMilestone: Retry clients\nBody.\n",
        )
        .unwrap();
        let tracker = linked_tracker(|s| s.milestones = vec![milestone(4, "x", false)]);
        let e = file_target(&mut db, &layout, &host, &tracker, Target::FollowUp(1)).unwrap_err();
        assert!(matches!(e, Error::MilestoneClosed(4)), "{e}");
        assert!(writes(&tracker, 0).is_empty());
        assert_eq!(field(&mut db, "issue").as_deref(), Some("12"));
    }

    /// RU-E-015 — a follow-up naming an absent milestone refuses, names it and files nothing.
    #[test]
    fn ut_follow_up_naming_an_absent_milestone_refuses() {
        let (_dir, mut db, layout, host) = linked_setup(&[("kind", "slice"), ("milestone", "4")]);
        std::fs::write(
            layout.run_dir("r").join("follow-up-1.md"),
            "# Follow title\n\nMilestone: Nowhere\nBody.\n",
        )
        .unwrap();
        let tracker = linked_tracker(|_| {});
        let e = file_target(&mut db, &layout, &host, &tracker, Target::FollowUp(1)).unwrap_err();
        assert!(
            matches!(&e, Error::Remote(remote::Error::NotFound(t)) if t == "Nowhere"),
            "{e}"
        );
        assert!(writes(&tracker, 0).is_empty());
    }

    /// RU-E-016 — a follow-up of a run that is no slice run is filed in no milestone, whatever
    /// its draft names.
    #[test]
    fn ut_follow_up_of_a_plain_run_has_no_milestone() {
        let (_dir, mut db, layout, host) = linked_setup(&[("milestone", "4")]);
        std::fs::write(
            layout.run_dir("r").join("follow-up-1.md"),
            "# Follow title\n\nMilestone: Retry clients\nBody.\n",
        )
        .unwrap();
        let tracker = linked_tracker(|_| {});
        file_target(&mut db, &layout, &host, &tracker, Target::FollowUp(1)).unwrap();
        let calls = tracker.calls();
        assert!(
            calls
                .iter()
                .any(|c| c.starts_with("create_issue Follow title milestone=- ")),
            "{calls:?}"
        );
        assert!(!calls.iter().any(|c| c.starts_with("list")), "{calls:?}");
    }

    /// RU-R-024 — a follow-up carries the labels of the areas its draft names, else the brief's.
    #[test]
    fn ut_follow_up_carries_its_area_labels() {
        let (_dir, mut db, layout, host) = linked_setup(&[]);
        let tracker = linked_tracker(|_| {});
        file_target(&mut db, &layout, &host, &tracker, Target::FollowUp(1)).unwrap();
        assert!(
            tracker
                .calls()
                .contains(&"create_issue Follow title milestone=- labels=area:runs".to_owned())
        );
        std::fs::write(
            layout.run_dir("r").join("follow-up-2.md"),
            "# Second title\n\nAreas: card\nBody.\n",
        )
        .unwrap();
        file_target(&mut db, &layout, &host, &tracker, Target::FollowUp(2)).unwrap();
        assert!(
            tracker
                .calls()
                .contains(&"create_issue Second title milestone=- labels=area:card".to_owned())
        );
    }

    /// RU-R-024 — an adopted follow-up gets only the labels it lacks.
    #[test]
    fn ut_adopted_follow_up_gets_only_missing_labels() {
        let (_dir, mut db, layout, host) = linked_setup(&[]);
        std::fs::write(
            layout.run_dir("r").join("follow-up-1.md"),
            "# Follow title\n\nAreas: runs, card\nBody of follow.\n",
        )
        .unwrap();
        let tracker = linked_tracker(|s| {
            s.open = vec![(30, "Follow title".into())];
            s.bodies.insert(30, "Body of follow.\n".into());
            s.links.insert(
                30,
                remote::IssueLinks {
                    open: true,
                    labels: vec!["area:runs".into()],
                    relates_to: vec![ItemRef::Number(12)],
                    ..Default::default()
                },
            );
        });
        file_target(&mut db, &layout, &host, &tracker, Target::FollowUp(1)).unwrap();
        assert_eq!(
            writes(&tracker, 0),
            ["ensure_labels area:card", "add_labels 30 area:card"]
        );
    }

    const CHANGE: &str = "Decision: keep retries\nSummary: retries stay\n\n@@ old\nold text\n@@ new\nnew text\n@@ end\n";

    /// A slice run on issue 12 in milestone 4 holding `description`, with a kept change.
    fn kept_setup(description: &str) -> (tempfile::TempDir, Board, Layout, FakeHost, FakeTracker) {
        let (dir, db, layout, host) = linked_setup(&[("kind", "slice"), ("milestone", "4")]);
        let run = layout.run_dir("r");
        std::fs::write(run.join("issue-comment.md"), "Kept R3.\n").unwrap();
        std::fs::write(run.join("milestone-change.md"), CHANGE).unwrap();
        let description = description.to_owned();
        let tracker = linked_tracker(|s| {
            s.login = "ann".into();
            s.milestones[0].description = description;
            s.links.insert(
                12,
                remote::IssueLinks {
                    open: true,
                    labels: vec!["area:runs".into()],
                    ..Default::default()
                },
            );
        });
        (dir, db, layout, host, tracker)
    }

    /// RU-R-020 — a kept change edits the milestone description in place and adds one changelog
    /// entry, once.
    #[test]
    fn ut_kept_change_updates_the_description() {
        let (_dir, mut db, layout, host, tracker) = kept_setup("Goal\nold text\n");
        file_target(&mut db, &layout, &host, &tracker, Target::Comment).unwrap();
        let d = tracker.0.borrow().milestones[0].description.clone();
        assert!(d.starts_with("Goal\nnew text\n"), "{d}");
        assert!(
            d.contains("2026-01-02T03:04 UTC · @ann · keep retries — retries stay"),
            "{d}"
        );
        // The comment goes to the run's issue alone, not an epic.
        assert_eq!(tracker.0.borrow().comments.len(), 1);
        db.conn_mut()
            .execute("DELETE FROM events WHERE name LIKE 'posted%'", [])
            .unwrap();
        let from = tracker.calls().len();
        file_target(&mut db, &layout, &host, &tracker, Target::Comment).unwrap();
        assert!(
            !tracker.calls()[from..]
                .iter()
                .any(|c| c.starts_with("set_description")),
            "{:?}",
            tracker.calls()
        );
    }

    /// RU-E-005, RU-R-020 — the description is read just before it is written, and the comment
    /// follows the write.
    #[test]
    fn ut_kept_change_reads_just_before_writing() {
        let (_dir, mut db, layout, host, tracker) = kept_setup("Goal\nold text\n");
        file_target(&mut db, &layout, &host, &tracker, Target::Comment).unwrap();
        let calls = tracker.calls();
        let i = calls.iter().position(|c| c == "get 4").unwrap();
        assert_eq!(
            calls[i + 1..i + 3],
            ["login", "set_description 4"],
            "{calls:?}"
        );
        let c = calls.iter().position(|c| c == "comment 12").unwrap();
        assert!(c > i + 2, "{calls:?}");
    }

    /// RU-E-011 — an old text no longer in the description refuses and leaves it unchanged.
    #[test]
    fn ut_stale_old_text_refuses_and_writes_nothing() {
        let (_dir, mut db, layout, host, tracker) = kept_setup("Goal\nsomething else\n");
        let e = file_target(&mut db, &layout, &host, &tracker, Target::Comment).unwrap_err();
        assert!(
            matches!(&e, Error::MilestoneStale(t) if t == "old text"),
            "{e}"
        );
        assert!(
            !tracker
                .calls()
                .iter()
                .any(|c| c.starts_with("set_description")),
            "{:?}",
            tracker.calls()
        );
        assert_eq!(
            tracker.0.borrow().milestones[0].description,
            "Goal\nsomething else\n"
        );
        assert!(tracker.0.borrow().comments.is_empty());
        assert!(!tracker.calls().iter().any(|c| c.starts_with("comment")));
    }

    /// RU-E-030 — a slice run without a readable change file refuses naming it and posts nothing.
    #[test]
    fn ut_kept_change_needs_a_readable_change_file() {
        let (_dir, mut db, layout, host, tracker) = kept_setup("Goal\nold text\n");
        let path = layout.run_dir("r").join("milestone-change.md");
        let untouched = |tracker: &FakeTracker| {
            assert!(tracker.0.borrow().comments.is_empty());
            assert!(
                !tracker
                    .calls()
                    .iter()
                    .any(|c| c.starts_with("comment") || c.starts_with("set_description")),
                "{:?}",
                tracker.calls()
            );
        };
        std::fs::write(&path, "no decision here\n").unwrap();
        let e = file_target(&mut db, &layout, &host, &tracker, Target::Comment).unwrap_err();
        assert!(matches!(&e, Error::Missing(p) if *p == path), "{e}");
        untouched(&tracker);
        std::fs::remove_file(&path).unwrap();
        let e = file_target(&mut db, &layout, &host, &tracker, Target::Comment).unwrap_err();
        assert!(matches!(&e, Error::Missing(p) if *p == path), "{e}");
        untouched(&tracker);
    }

    fn set_descriptions(tracker: &FakeTracker) -> usize {
        tracker
            .calls()
            .iter()
            .filter(|c| c.starts_with("set_description"))
            .count()
    }

    /// RU-R-020 — a comment post reports the milestone outcome: edited, then recorded.
    #[test]
    fn ut_comment_reports_the_milestone_outcome() {
        let (_dir, mut db, layout, host, tracker) = kept_setup("Goal\nold text\n");
        assert_eq!(
            comment_on(&mut db, &layout, &host, &tracker).unwrap(),
            Commented {
                issue: ItemRef::Number(12),
                action: Action::Commented,
                milestone: Some((4, Action::Edited)),
            }
        );
        let again = comment_on(&mut db, &layout, &host, &tracker).unwrap();
        assert_eq!(again.action, Action::Recorded);
        assert_eq!(again.milestone, Some((4, Action::Recorded)));
        assert_eq!(tracker.0.borrow().comments.len(), 1);
        assert_eq!(set_descriptions(&tracker), 1);
    }

    /// RU-R-020, RU-R-021 — a failed comment keeps the written change; the rerun posts the
    /// comment alone.
    #[test]
    fn ut_failed_comment_rerun_posts_it_alone() {
        let (_dir, mut db, layout, host, tracker) = kept_setup("Goal\nold text\n");
        tracker.0.borrow_mut().fail_on = Some("comment");
        comment_on(&mut db, &layout, &host, &tracker).unwrap_err();
        let d = tracker.0.borrow().milestones[0].description.clone();
        assert!(d.contains("new text"), "{d}");
        assert_eq!(d.matches("keep retries — retries stay").count(), 1, "{d}");
        assert!(tracker.0.borrow().comments.is_empty());
        tracker.0.borrow_mut().fail_on = None;
        let done = comment_on(&mut db, &layout, &host, &tracker).unwrap();
        assert_eq!(done.milestone, Some((4, Action::Recorded)));
        assert_eq!(done.action, Action::Commented);
        assert_eq!(tracker.0.borrow().comments.len(), 1);
        assert_eq!(set_descriptions(&tracker), 1);
        let d = tracker.0.borrow().milestones[0].description.clone();
        assert_eq!(d.matches("keep retries — retries stay").count(), 1, "{d}");
    }

    /// RU-R-020 — a failed description write posts no comment; the rerun does both.
    #[test]
    fn ut_failed_milestone_write_posts_no_comment() {
        let (_dir, mut db, layout, host, tracker) = kept_setup("Goal\nold text\n");
        tracker.0.borrow_mut().fail_on = Some("set_description");
        comment_on(&mut db, &layout, &host, &tracker).unwrap_err();
        assert!(tracker.0.borrow().comments.is_empty());
        assert_eq!(
            tracker.0.borrow().milestones[0].description,
            "Goal\nold text\n"
        );
        tracker.0.borrow_mut().fail_on = None;
        let done = comment_on(&mut db, &layout, &host, &tracker).unwrap();
        assert_eq!(done.milestone, Some((4, Action::Edited)));
        assert_eq!(done.action, Action::Commented);
        assert_eq!(tracker.0.borrow().comments.len(), 1);
    }

    /// RU-R-020 — a comment on a run that is no slice run in a milestone reports no milestone.
    #[test]
    fn ut_comment_off_a_milestone_reports_none() {
        let (_dir, mut db, layout, host) = linked_setup(&[]);
        std::fs::write(layout.run_dir("r").join("issue-comment.md"), "Note.\n").unwrap();
        let tracker = linked_tracker(|s| {
            s.links.insert(
                12,
                remote::IssueLinks {
                    open: true,
                    labels: vec!["area:runs".into()],
                    ..Default::default()
                },
            );
        });
        let done = comment_on(&mut db, &layout, &host, &tracker).unwrap();
        assert_eq!(done.milestone, None);
        assert_eq!(done.action, Action::Commented);
        assert!(!tracker.calls().iter().any(|c| c.starts_with("get")));
    }

    /// RU-R-036 — a kept change adds the label of an area the brief now names, none removed.
    #[test]
    fn ut_kept_change_adds_a_new_area_label() {
        let (_dir, mut db, layout, host, tracker) = kept_setup("Goal\nold text\n");
        std::fs::write(
            layout.run_dir("r").join("spec-diff.md"),
            "Area: runs — a; card — b\n",
        )
        .unwrap();
        tracker.0.borrow_mut().links.get_mut(&12).unwrap().labels =
            vec!["area:runs".into(), "bug".into()];
        file_target(&mut db, &layout, &host, &tracker, Target::Comment).unwrap();
        let calls = tracker.calls();
        assert!(
            calls.contains(&"add_labels 12 area:card".to_owned()),
            "{calls:?}"
        );
        assert!(!calls.iter().any(|c| c.contains("add_labels 12 area:runs")));
    }

    /// A tracker without milestones gets the split-off's marker, its `Split from` line and a
    /// note on the epic.
    #[test]
    fn ut_prose_split_keeps_its_marker_and_line() {
        let (_dir, mut db, layout, host) = linked_setup(&[("kind", "slice"), ("epic", "#9")]);
        let tracker = tracker_with(|s| {
            s.no_milestones = true;
            s.next = 8;
            s.open = vec![(30, "Split title".into())];
        });
        assert_eq!(
            file_target(&mut db, &layout, &host, &tracker, Target::Split).unwrap(),
            (ItemRef::Number(30), Action::Adopted)
        );
        let body = tracker.0.borrow().bodies[&30].clone();
        assert!(
            body.starts_with("**Part of epic** #9 · split from #12\n\n"),
            "{body}"
        );
        assert!(body.ends_with("\n\nSplit from #12\n"), "{body}");
        tracker.0.borrow_mut().open.clear();
        board::set(db.conn_mut(), "r", "split", "").unwrap();
        db.conn_mut()
            .execute("DELETE FROM events WHERE name LIKE 'posted%'", [])
            .unwrap();
        file_target(&mut db, &layout, &host, &tracker, Target::Split).unwrap();
        assert!(tracker.calls().contains(&"comment 9".to_owned()));
        assert!(!tracker.calls().iter().any(|c| c.starts_with("set_parent")));
    }

    /// A tracker without milestones gets the follow-up's `Found in` line and no relationship.
    #[test]
    fn ut_prose_follow_up_keeps_its_found_in_line() {
        let (_dir, mut db, layout, host) = linked_setup(&[]);
        let tracker = tracker_with(|s| {
            s.no_milestones = true;
            s.open = vec![(30, "Follow title".into())];
        });
        file_target(&mut db, &layout, &host, &tracker, Target::FollowUp(1)).unwrap();
        let body = tracker.0.borrow().bodies[&30].clone();
        assert!(body.ends_with("\n\nFound in #12\n"), "{body}");
        assert!(
            !tracker
                .calls()
                .iter()
                .any(|c| c.starts_with("add_relates_to"))
        );
    }

    /// A tracker without milestones comments on the epic as well, and a rerun after the issue's
    /// comment was logged posts only the epic's.
    #[test]
    fn ut_prose_comment_retries_only_the_epic() {
        let (_dir, mut db, layout, host) =
            setup(&[("issue", "12"), ("kind", "slice"), ("epic", "#9")]);
        std::fs::write(layout.run_dir("r").join("issue-comment.md"), "Kept R3.\n").unwrap();
        let tracker = tracker_with(|s| s.no_milestones = true);
        let text = "Kept R3.\n";
        let sum = crate::install::hash(text);
        board::log(
            db.conn_mut(),
            "r",
            "posted=comment",
            &["issue=12".to_owned(), format!("sum={sum}")],
            "t",
        )
        .unwrap();
        assert_eq!(
            file_target(&mut db, &layout, &host, &tracker, Target::Comment).unwrap(),
            (ItemRef::Number(12), Action::Commented)
        );
        assert_eq!(tracker.calls(), ["view 9", "comment 9"]);
    }
}
