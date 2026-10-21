//! What a run posts to its tracker and CI host: the tracking issue, slice,
//! split-off and follow-up issues, issue comments, the draft PR and its
//! body. Every body is rendered in memory, stripped of attribution lines
//! and posted; the number posted to is recorded on the parent card. An
//! open issue already titled exactly as the one to file is adopted, never
//! filed twice, and an open PR of the run's branch is adopted, never
//! opened twice, so a run resumed after a crash cannot duplicate one. Each
//! comment target and follow-up done is logged (`posted=`), so a rerun
//! after a partial failure does only what is missing.

use std::fmt::Write as _;
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
use crate::run::gauntlet::{self, Outcome};
use crate::store::{Board, Reader, snapshot};
use crate::{git, remote, run, spec};

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
            Error::Missing(_) | Error::NoIssue | Error::NoBranch | Error::Card | Error::NoPr => {
                None
            }
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

fn list(numbers: &[u64]) -> String {
    let items: Vec<String> = numbers.iter().map(u64::to_string).collect();
    format!("[{}]", items.join(", "))
}

/// `[71, -, 73]` by position: item n is slice n's issue, `-` one not
/// filed yet.
#[must_use]
pub fn slots(list: Option<&str>) -> Vec<Option<u64>> {
    let inner = list
        .unwrap_or("")
        .trim()
        .trim_matches(|c| c == '[' || c == ']');
    if inner.trim().is_empty() {
        return Vec::new();
    }
    inner
        .split(',')
        .map(|n| n.trim().trim_start_matches('#').parse().ok())
        .collect()
}

fn slot_list(slots: &[Option<u64>]) -> String {
    let items: Vec<String> = slots
        .iter()
        .map(|n| n.map_or_else(|| "-".to_owned(), |n| n.to_string()))
        .collect();
    format!("[{}]", items.join(", "))
}

/// A `posted=` event's issue, sum and k.
type Posted = (Option<u64>, Option<String>, Option<u32>);

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
    fn field(&self, key: &str) -> rusqlite::Result<Option<String>> {
        Ok(board::field(self.conn, self.slug, key)?.filter(|v| !v.is_empty()))
    }

    fn set(&mut self, key: &str, value: &str) -> Result<(), Error> {
        board::set(self.conn, self.slug, key, value).map_err(|e| match e {
            board::CardError::Sql(e) => Error::Sql(e),
            _ => Error::Card,
        })
    }

    fn log(&mut self, event: &str, args: &[String]) -> Result<(), Error> {
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
                } if w == what => Some((issue, sum.map(str::to_owned), k)),
                _ => None,
            })
            .collect())
    }

    fn issue(&self) -> Result<u64, Error> {
        numbers(self.field("issue")?.as_deref())
            .first()
            .copied()
            .ok_or(Error::NoIssue)
    }

    /// Adopts the open issue titled `title`, else files a new one.
    fn adopt_or_create(&self, title: &str, body: &str) -> Result<(u64, Action), Error> {
        if let Some(n) = self.tracker.find_open(title)? {
            return Ok((n, Action::Adopted));
        }
        Ok((self.tracker.create(title, body)?, Action::Created))
    }

    /// Posts `target`; the issue posted to and what happened.
    pub fn file(&mut self, target: Target, reuse: Option<u64>) -> Result<(u64, Action), Error> {
        if board::state(self.conn, self.slug)?.is_none() {
            return Err(Error::Card);
        }
        match target {
            Target::Run => self.file_run(reuse),
            Target::Slice(n) => self.file_slice(n),
            Target::Split => self.file_split(),
            Target::FollowUp(k) => self.file_follow_up(k),
            Target::Comment => self.file_comment(),
        }
    }

    /// follow-up-k.md, filed once: a `posted=follow-up k=<k>` names the
    /// issue it became.
    fn file_follow_up(&mut self, k: u32) -> Result<(u64, Action), Error> {
        let issue = self.issue()?;
        if let Some(n) = self
            .posted("follow-up")?
            .into_iter()
            .find_map(|(n, _, kk)| if kk == Some(k) { n } else { None })
        {
            return Ok((n, Action::Recorded));
        }
        let (title, body) = split_title(&self.artifact(&format!("follow-up-{k}.md"))?);
        let body = format!("{}\n\nFound in #{issue}\n", body.trim_end());
        let (n, action) = self.adopt_or_create(&title, &body)?;
        self.log(
            "posted=follow-up",
            &[format!("issue={n}"), format!("k={k}")],
        )?;
        Ok((n, action))
    }

    /// issue-comment.md on the tracking issue and a slice's epic, each
    /// target once per text: a `posted=comment issue=<n> sum=<hash>`
    /// marks one done, so a rerun after a failure posts only the rest.
    fn file_comment(&mut self) -> Result<(u64, Action), Error> {
        let issue = self.issue()?;
        let text = self.artifact("issue-comment.md")?;
        let text = attribution::strip(&text).unwrap_or(text);
        let sum = crate::install::hash(&text);
        let mut targets = vec![issue];
        if self.field("kind")?.as_deref() == Some("slice")
            && let Some(&epic) = numbers(self.field("epic")?.as_deref()).first()
        {
            targets.push(epic);
        }
        let done = self.posted("comment")?;
        let mut action = Action::Recorded;
        for t in targets {
            if done
                .iter()
                .any(|(n, s, _)| *n == Some(t) && s.as_deref() == Some(sum.as_str()))
            {
                continue;
            }
            self.tracker.comment(&ItemRef::Number(t), &text)?;
            self.log(
                "posted=comment",
                &[format!("issue={t}"), format!("sum={sum}")],
            )?;
            action = Action::Commented;
        }
        Ok((issue, action))
    }

    fn file_run(&mut self, reuse: Option<u64>) -> Result<(u64, Action), Error> {
        let rendered = snapshot(self.conn, |r| body::issue(r, self.layout, self.slug, None))?;
        let (title, body) = split_title(&rendered);
        let current = numbers(self.field("issue")?.as_deref()).first().copied();
        let (n, action) = match (reuse, current) {
            (Some(n), _) => {
                self.tracker.edit(&ItemRef::Number(n), &title, &body)?;
                (n, Action::Edited)
            }
            (None, Some(n)) if self.field("kind")?.as_deref() == Some("epic") => {
                self.tracker.edit(&ItemRef::Number(n), &title, &body)?;
                (n, Action::Edited)
            }
            (None, Some(n)) => (n, Action::Recorded),
            (None, None) => self.adopt_or_create(&title, &body)?,
        };
        self.set("issue", &n.to_string())?;
        Ok((n, action))
    }

    fn file_slice(&mut self, slice: u32) -> Result<(u64, Action), Error> {
        let Some(at) = (slice as usize).checked_sub(1) else {
            return Err(Error::Body(body::Error::NoSlice(slice)));
        };
        let mut slots = slots(self.field("slices")?.as_deref());
        if let Some(&Some(n)) = slots.get(at) {
            return Ok((n, Action::Recorded));
        }
        let rendered = snapshot(self.conn, |r| {
            body::issue(r, self.layout, self.slug, Some(slice))
        })?;
        let (title, body) = split_title(&rendered);
        let (n, action) = self.adopt_or_create(&title, &body)?;
        if slots.len() <= at {
            slots.resize(at + 1, None);
        }
        slots[at] = Some(n);
        self.set("slices", &slot_list(&slots))?;
        Ok((n, action))
    }

    fn file_split(&mut self) -> Result<(u64, Action), Error> {
        let issue = self.issue()?;
        let (title, body) = split_title(&self.artifact("split-issue.md")?);
        let epic = if self.field("kind")?.as_deref() == Some("slice") {
            numbers(self.field("epic")?.as_deref()).first().copied()
        } else {
            None
        };
        let mut text = String::new();
        if let Some(e) = epic {
            let _ = write!(text, "**Part of epic** #{e} · split from #{issue}\n\n");
        }
        text.push_str(body.trim_end());
        let _ = write!(text, "\n\nSplit from #{issue}\n");
        let (n, action) = self.adopt_or_create(&title, &text)?;
        if let (Some(e), Action::Created) = (epic, action) {
            let note = format!("Split off #{n} from #{issue}.\n");
            self.tracker.comment(&ItemRef::Number(e), &note)?;
        }
        let mut split = numbers(self.field("split")?.as_deref());
        if !split.contains(&n) {
            split.push(n);
            self.set("split", &list(&split))?;
        }
        Ok((n, action))
    }

    /// Opens the draft PR from the rendered draft body unless the card has
    /// one; its number and whether it was opened (or adopted) now. An open
    /// PR of the branch, one a crash left unrecorded, is adopted and its
    /// body synced, never opened twice.
    pub fn open_pr(&mut self) -> Result<(u64, bool), Error> {
        if board::state(self.conn, self.slug)?.is_none() {
            return Err(Error::Card);
        }
        if let Some(&n) = numbers(self.field("pr")?.as_deref()).first() {
            return Ok((n, false));
        }
        let branch = self.field("branch")?.ok_or(Error::NoBranch)?;
        let rendered = snapshot(self.conn, |r| {
            body::pr(
                r,
                self.layout,
                self.slug,
                Form::Draft,
                &self.config.main_branch,
            )
        })?;
        let (title, body) = split_title(&rendered);
        let adopted = self.host.find_open(&branch)?;
        let n = match adopted {
            Some(n) => n,
            None => self
                .host
                .create_draft(&branch, &self.config.main_branch, &title, &body)?,
        };
        self.set("pr", &n.to_string())?;
        if !board::events(self.conn, self.slug, &[Name::Pr])?
            .iter()
            .any(|e| e.typed() == Typed::Pr(PrState::Draft))
        {
            self.log("pr=draft", &[])?;
        }
        if adopted.is_some() {
            self.sync_draft()?;
        }
        Ok((n, true))
    }

    /// Re-renders the draft body and pushes it when it differs from the
    /// PR's; `None` while the run has no PR.
    pub fn sync_draft(&mut self) -> Result<Option<Sync>, Error> {
        let Some(&n) = numbers(self.field("pr")?.as_deref()).first() else {
            return Ok(None);
        };
        let rendered = match snapshot(self.conn, |r| {
            body::pr(
                r,
                self.layout,
                self.slug,
                Form::Draft,
                &self.config.main_branch,
            )
        }) {
            Ok(r) => r,
            Err(body::Error::StageRef(t)) => return Ok(Some(Sync::Refused(t))),
            Err(e) => return Err(e.into()),
        };
        let (_, body) = split_title(&rendered);
        let pr = ItemRef::Number(n);
        let current = self.host.status(&pr)?.body;
        if current.trim_end() == body.trim_end() {
            return Ok(Some(Sync::Unchanged));
        }
        self.host.edit(&pr, None, &body)?;
        Ok(Some(Sync::Pushed))
    }
}

/// Why `pr push` pushed nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PushRefused {
    /// The run's branch is the main branch.
    Main,
    /// The card has no branch.
    NoBranch,
    /// The worktree's head is no stage's verified commit: the mismatch.
    Sha(String),
    /// The worktree holds changes no commit has.
    Dirty,
    /// The gauntlet did not pass: its outcome.
    Gauntlet(Outcome),
    /// `git push` failed.
    Push(git::GitError),
}

/// The fix when `git push` to origin fails.
pub const PUSH_HINT: &str =
    "git push to origin failed (network, auth or a rejected update); retry once, then ask the user";

impl PushRefused {
    /// The `push=refused` line.
    #[must_use]
    pub fn refusal(&self) -> Refusal {
        match self {
            PushRefused::Main => {
                Refusal::new("push", "main").hint("push from a branch other than main")
            }
            PushRefused::NoBranch => Refusal::new("push", "branch")
                .hint("approve gate2 first: ferrule gate approve <slug> gate2"),
            PushRefused::Sha(m) => Refusal::new("push", "sha")
                .field("mismatch", m)
                .hint("push a stage's last commit= on the run's branch: ferrule stage verify"),
            PushRefused::Dirty => Refusal::new("push", "dirty")
                .hint("commit the changes through `ferrule stage commit`, then push again"),
            PushRefused::Gauntlet(outcome) => match outcome {
                Outcome::Fail {
                    step,
                    sha,
                    timeout,
                    log,
                } => {
                    let r = Refusal::new("push", "gauntlet")
                        .field("step", step)
                        .field("sha", sha);
                    let r = if *timeout {
                        r.field("timeout", "yes")
                    } else {
                        r
                    };
                    r.field("log", log.display())
                        .hint("fix the failing step, commit, push again")
                }
                other => other.refusal("push").unwrap_or_else(|| {
                    Refusal::new("push", "gauntlet")
                        .hint("fix the failing step, commit, push again")
                }),
            },
            PushRefused::Push(e) => Refusal::new("push", "push")
                .detail(&e.stderr)
                .hint(PUSH_HINT),
        }
    }
}

impl std::fmt::Display for PushRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.refusal())
    }
}

/// What `pr push` did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pushed {
    pub sha: String,
    pub cached: bool,
    /// The draft body's sync, or the PR opened by this first push.
    pub body: Result<Sync, u64>,
}

impl std::fmt::Display for Pushed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "push=done sha={} gauntlet=pass", self.sha)?;
        if self.cached {
            f.write_str(" cached")?;
        }
        match &self.body {
            Ok(sync) => write!(f, " {sync}"),
            Err(n) => write!(f, " pr-body=opened pr={n}"),
        }
    }
}

/// Why `pr publish` did not mark the PR ready.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PublishRefused {
    /// Gate 4 is not approved: the parent card has no `gate4` date.
    Gate4,
    /// Unresolved review threads: their count.
    Threads(usize),
    /// Fixed IDs another run landed on main meanwhile.
    Collide(Vec<String>),
    /// Main could not be fetched, or the branch shares no history with it.
    CollisionsUnknown,
    /// The full body names a stage or an agent.
    StageRef(String),
}

impl PublishRefused {
    /// The `publish=refused` line; a stage reference exits 3.
    #[must_use]
    pub fn refusal(&self) -> Refusal {
        match self {
            PublishRefused::Gate4 => Refusal::new("publish", "gate4")
                .hint("approve gate4 first: ferrule gate approve <slug> gate4"),
            PublishRefused::Threads(n) => Refusal::new("publish", "threads")
                .field("threads", n)
                .hint("resolve the open threads: ferrule guide show feedback"),
            PublishRefused::Collide(ids) => Refusal::new("publish", "collide")
                .field("ids", ids.join(","))
                .hint("renumber the colliding IDs on the branch, push, publish again"),
            PublishRefused::CollisionsUnknown => Refusal::new("publish", "collisions-unknown")
                .hint(
                    "main could not be fetched or shares no history with the branch; \
                 ask the user to fix the remote, then publish again",
                ),
            PublishRefused::StageRef(t) => Refusal::new("publish", "stage-ref")
                .field("stage-ref", t)
                .hint("reword the text naming it, then publish again")
                .exit(Exit::Content),
        }
    }
}

impl std::fmt::Display for PublishRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.refusal())
    }
}

impl Run<'_> {
    fn worktree(&self) -> rusqlite::Result<PathBuf> {
        crate::run::worktree(self.conn, self.layout, self.slug, self.slug)
    }

    /// Pushes the worktree's head once it is a stage's verified commit and
    /// the gauntlet passes on it, logs the pass, and updates the draft PR
    /// (the first push opens it).
    pub fn push(&mut self) -> Result<Result<Pushed, PushRefused>, Error> {
        if board::state(self.conn, self.slug)?.is_none() {
            return Err(Error::Card);
        }
        let Some(branch) = self.field("branch")? else {
            return Ok(Err(PushRefused::NoBranch));
        };
        if branch == self.config.main_branch {
            return Ok(Err(PushRefused::Main));
        }
        let wt = self.worktree()?;
        if gauntlet::dirty(&wt) {
            return Ok(Err(PushRefused::Dirty));
        }
        let Some(head) = git::out(&wt, &["rev-parse", "HEAD"]) else {
            return Ok(Err(PushRefused::Sha("missing".to_owned())));
        };
        let mut stage = None;
        for (n, id, _) in board::stages(self.conn, self.slug)? {
            if let Some(c) = board::last_value(self.conn, &id, Name::Commit)?
                && head.starts_with(&c)
            {
                stage = Some(n);
            }
        }
        let Some(stage) = stage else {
            return Ok(Err(PushRefused::Sha("card-commit".to_owned())));
        };
        if let Err(m) = run::sha_check(
            &Reader::new(self.conn),
            self.layout,
            self.slug,
            &format!("s{stage}"),
            &head,
        )? {
            return Ok(Err(PushRefused::Sha(m.as_str().to_owned())));
        }
        let log = self.layout.run_dir(self.slug).join("gauntlet.log");
        let outcome = gauntlet::run_in(
            self.conn,
            self.slug,
            &wt,
            &log,
            &self.config.gauntlet.step,
            false,
            self.now,
        )
        .map_err(|e| Error::Remote(remote::Error::Command(e.to_string())))?;
        let Outcome::Pass {
            cov, sha, cached, ..
        } = outcome
        else {
            return Ok(Err(PushRefused::Gauntlet(outcome)));
        };
        if let Err(e) = git::run(&wt, &["push", "-q", "-u", "origin", &branch]) {
            return Ok(Err(PushRefused::Push(e)));
        }
        self.log(
            "gauntlet=pass",
            &[format!("cov={cov}"), format!("sha={sha}")],
        )?;
        let body = match self.sync_draft()? {
            Some(sync) => Ok(sync),
            None => Err(self.open_pr()?.0),
        };
        Ok(Ok(Pushed { sha, cached, body }))
    }

    /// Gate 4 approved (refused without its `gate4` date): checks the
    /// threads and the fixed IDs, replaces the draft's title and body with
    /// the full form, marks it ready, logs `pr=ready` and files the kept
    /// follow-ups (`None`: all of them). Rerun after a failure, a PR that
    /// is ready with `pr=ready` logged is left alone and a follow-up
    /// already filed is not filed again.
    pub fn publish(
        &mut self,
        keep: Option<&[u32]>,
    ) -> Result<Result<(u64, Vec<u64>), PublishRefused>, Error> {
        if board::state(self.conn, self.slug)?.is_none() {
            return Err(Error::Card);
        }
        if self.field("gate4")?.is_none() {
            return Ok(Err(PublishRefused::Gate4));
        }
        let pr = match numbers(self.field("pr")?.as_deref()).first() {
            Some(&n) => n,
            None => self.open_pr()?.0,
        };
        let pr_ref = ItemRef::Number(pr);
        let threads = self.host.threads(&pr_ref)?.threads.len();
        if threads > 0 {
            return Ok(Err(PublishRefused::Threads(threads)));
        }
        let wt = self.worktree()?;
        let main = self.config.main_branch.clone();
        let fetched = git::ok(&wt, &["fetch", "-q", "origin", &main]);
        let hits = match spec::collisions(&wt, &format!("origin/{main}")) {
            Some(hits) if fetched => hits,
            _ => return Ok(Err(PublishRefused::CollisionsUnknown)),
        };
        if !hits.is_empty() {
            return Ok(Err(PublishRefused::Collide(hits)));
        }
        let rendered = match snapshot(self.conn, |r| {
            body::pr(r, self.layout, self.slug, Form::Full, &main)
        }) {
            Ok(r) => r,
            Err(body::Error::StageRef(t)) => return Ok(Err(PublishRefused::StageRef(t))),
            Err(e) => return Err(e.into()),
        };
        let (title, body) = split_title(&rendered);
        let logged = board::events(self.conn, self.slug, &[Name::Pr])?
            .iter()
            .any(|e| e.typed() == Typed::Pr(PrState::Ready));
        let ready = logged && {
            let s = self.host.status(&pr_ref)?;
            s.state == "open" && !s.draft
        };
        if !ready {
            self.host.edit(&pr_ref, Some(&title), &body)?;
            self.host.ready(&pr_ref)?;
        }
        if !logged {
            self.log("pr=ready", &[])?;
        }
        let mut filed = Vec::new();
        let mut new = false;
        for k in follow_up_numbers(&self.layout.run_dir(self.slug)) {
            if keep.is_some_and(|keep| !keep.contains(&k)) {
                continue;
            }
            let (n, action) = self.file(Target::FollowUp(k), None)?;
            new |= action != Action::Recorded;
            filed.push(n);
        }
        if new {
            let list: Vec<String> = filed.iter().map(u64::to_string).collect();
            self.log(&format!("follow-ups=[{}]", list.join(",")), &[])?;
        }
        Ok(Ok((pr, filed)))
    }

    /// Waits for the PR's checks; the exit code, the failed checks and the
    /// output, also written to the run's ci.log.
    pub fn wait_ci(&self) -> Result<(i32, Vec<String>), Error> {
        if board::state(self.conn, self.slug)?.is_none() {
            return Err(Error::Card);
        }
        let pr = numbers(self.field("pr")?.as_deref())
            .first()
            .copied()
            .ok_or(Error::NoPr)?;
        let (code, output) = self.host.checks_watch(&ItemRef::Number(pr))?;
        std::fs::write(self.layout.run_dir(self.slug).join("ci.log"), &output)
            .map_err(|e| Error::Remote(remote::Error::Command(e.to_string())))?;
        Ok((code, remote::failed_checks(&output)))
    }
}

impl Run<'_> {
    /// Fetches the PR's open threads into the run's pr-feedback.md and, with
    /// any, logs `feedback=<count>` and moves the stages owning their files
    /// back to inprogress (a file no stage lists goes to the last stage).
    /// The PR, the thread count, the stages moved and, when any moved, the
    /// draft body's sync.
    pub fn route_feedback(&mut self) -> Result<(u64, usize, Vec<u32>, Option<Sync>), Error> {
        if board::state(self.conn, self.slug)?.is_none() {
            return Err(Error::Card);
        }
        let pr = numbers(self.field("pr")?.as_deref())
            .first()
            .copied()
            .ok_or(Error::NoPr)?;
        let fetched = self.host.threads(&ItemRef::Number(pr))?;
        let file = self.layout.run_dir(self.slug).join("pr-feedback.md");
        std::fs::write(&file, fetched.file(self.now))
            .map_err(|e| Error::Remote(remote::Error::Command(e.to_string())))?;
        let count = fetched.threads.len();
        if count == 0 {
            return Ok((pr, 0, Vec::new(), None));
        }
        self.log(&format!("feedback={count}"), &[])?;
        let stages = board::stages(self.conn, self.slug)?;
        let mut owners: Vec<u32> = Vec::new();
        for t in &fetched.threads {
            let path = t.path.rsplit_once(':').map_or(t.path.as_str(), |(p, _)| p);
            let mut owner = stages.last().map(|(n, _, _)| *n);
            for (n, id, _) in &stages {
                let files = board::field(self.conn, id, "files")?.unwrap_or_default();
                if files
                    .trim_matches(|c| c == '[' || c == ']')
                    .split(',')
                    .any(|f| f.trim() == path)
                {
                    owner = Some(*n);
                    break;
                }
            }
            if let Some(n) = owner
                && !owners.contains(&n)
            {
                owners.push(n);
            }
        }
        owners.sort_unstable();
        for n in &owners {
            match board::move_card(
                self.conn,
                &format!("{}.s{n}", self.slug),
                board::State::InProgress,
                self.now,
            ) {
                Ok(()) => {}
                Err(board::MoveError::Sql(e)) => return Err(Error::Sql(e)),
                Err(_) => return Err(Error::Card),
            }
        }
        let sync = if owners.is_empty() {
            None
        } else {
            self.sync_draft()?
        };
        Ok((pr, count, owners, sync))
    }

    /// What git cannot answer about the run: the issue open and carrying a
    /// comment per kept spec change, the PR's state against the log, and
    /// its body against the board. One `differ` row each; a check the
    /// configured host cannot make ([`remote::Error::Unsupported`]) is
    /// skipped, not reported.
    pub fn reconcile_remote(&self) -> Result<Vec<String>, Error> {
        let mut differ = Vec::new();
        if let Some(&n) = numbers(self.field("issue")?.as_deref()).first() {
            match self.tracker.view(&ItemRef::Number(n)) {
                Ok(item) => {
                    if !item.open {
                        differ.push(format!("issue #{n} is not open"));
                    }
                    let kept = board::events(self.conn, self.slug, &[Name::SpecKeep])?.len();
                    if item.comments.len() < kept {
                        differ.push(format!(
                            "issue #{n} has {} comments for {kept} kept spec changes",
                            item.comments.len()
                        ));
                    }
                }
                Err(remote::Error::Unsupported { .. }) => {}
                Err(e) => differ.push(format!("issue #{n} unreadable: {e}")),
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
                    if state != "open" {
                        differ.push(format!("pr #{n} is {state}"));
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
    let mut ks: Vec<u32> = std::fs::read_dir(dir)
        .map(|d| {
            d.filter_map(Result::ok)
                .filter_map(|e| {
                    let name = e.file_name().to_string_lossy().into_owned();
                    name.strip_prefix("follow-up-")?
                        .strip_suffix(".md")?
                        .parse()
                        .ok()
                })
                .collect()
        })
        .unwrap_or_default();
    ks.sort_unstable();
    ks
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
        assert_eq!(run.file(Target::Run, None).unwrap(), (31, Action::Adopted));
        assert_eq!(run.file(Target::Run, None).unwrap(), (31, Action::Recorded));
        assert_eq!(
            tracker.calls(),
            ["find_open Retry clients"],
            "adopted by its exact title, never filed"
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
        assert_eq!(run.file(Target::Run, None).unwrap(), (8, Action::Created));
        assert_eq!(run.file(Target::Run, None).unwrap(), (8, Action::Recorded));
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
            (12, Action::Commented)
        );
        assert_eq!(
            run.file(Target::Comment, None).unwrap(),
            (12, Action::Recorded)
        );
        assert_eq!(tracker.calls(), ["comment 12", "comment 9"]);
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
        assert_eq!(list(&[1, 2]), "[1, 2]");
    }

    #[test]
    fn ut_slots_keep_positions() {
        assert_eq!(slots(Some("[71, -, #73]")), [Some(71), None, Some(73)]);
        assert!(slots(Some("[]")).is_empty());
        assert!(slots(None).is_empty());
        assert_eq!(slot_list(&[None, Some(72)]), "[-, 72]");
    }
}
