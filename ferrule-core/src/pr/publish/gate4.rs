//! Gate 4: the PR made ready with the follow-ups filed, and its checks
//! waited for.

use super::{
    Action, Error, Exit, Form, ItemRef, Name, PrState, Refusal, Run, Target, Typed, board, body,
    follow_up_numbers, git, numbers, remote, snapshot, spec, split_title,
};

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
    /// Gate 4 approved (refused without its `gate4` date): checks the
    /// threads and the fixed IDs, replaces the draft's title and body with
    /// the full form, marks it ready, logs `pr=ready` and files the kept
    /// follow-ups (`None`: all of them). Rerun after a failure, a PR that
    /// is ready with `pr=ready` logged is left alone and a follow-up
    /// already filed is not filed again.
    pub fn publish(
        &mut self,
        keep: Option<&[u32]>,
    ) -> Result<Result<(u64, Vec<ItemRef>), PublishRefused>, Error> {
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
        // Logged and ready, or merged or closed since: nothing to mark, the
        // follow-ups still to file.
        let ready = logged && {
            let s = self.host.status(&pr_ref)?;
            s.state != remote::PrState::Open || !s.draft
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
            let list: Vec<String> = filed.iter().map(ToString::to_string).collect();
            self.log(&format!("follow-ups=[{}]", list.join(",")), &[])?;
        }
        Ok(Ok((pr, filed)))
    }

    /// Waits for the PR's checks and what they came to; their output is
    /// written to the run's ci.log.
    pub fn wait_ci(&self) -> Result<remote::Checks, Error> {
        if board::state(self.conn, self.slug)?.is_none() {
            return Err(Error::Card);
        }
        let pr = numbers(self.field("pr")?.as_deref())
            .first()
            .copied()
            .ok_or(Error::NoPr)?;
        let checks = self.host.checks_watch(&ItemRef::Number(pr))?;
        std::fs::write(self.layout.run_dir(self.slug).join("ci.log"), &checks.raw)
            .map_err(|e| Error::Remote(remote::Error::Command(e.to_string())))?;
        Ok(checks)
    }
}
