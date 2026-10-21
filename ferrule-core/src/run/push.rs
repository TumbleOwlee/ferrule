//! `pr push`: a stage's verified commit pushed once the gauntlet passes on
//! it, the pass logged and the draft PR opened or updated.

use crate::card::board;
use crate::card::typed::Name;
use crate::git;
use crate::pr::publish::{Error, Run, Sync};
use crate::refusal::Refusal;
use crate::remote;
use crate::run::gauntlet::{self, Outcome};
use crate::store::Reader;

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

impl Run<'_> {
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
                && board::same_commit(&head, &c)
            {
                stage = Some(n);
            }
        }
        let Some(stage) = stage else {
            return Ok(Err(PushRefused::Sha("card-commit".to_owned())));
        };
        if let Err(m) = crate::run::sha_check(
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
}
