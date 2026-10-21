//! PR review feedback routed back to the stages that own the commented
//! files.

use super::{Error, ItemRef, Run, Sync, board, numbers, remote};

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
        let mut text = fetched.file(self.now);
        if let Ok(old) = std::fs::read_to_string(&file) {
            text = crate::pr::feedback::carry_over(&old, &text);
        }
        std::fs::write(&file, text)
            .map_err(|e| Error::Remote(remote::Error::Command(e.to_string())))?;
        let count = fetched.threads.len();
        if count == 0 {
            return Ok((pr, 0, Vec::new(), None));
        }
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
        // The stages back to inprogress and the count in one transaction,
        // the count last: a refused move logs nothing and moves nothing.
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let w = crate::store::Writer::new(&tx);
        for n in &owners {
            match w.move_card(
                &format!("{}.s{n}", self.slug),
                board::State::InProgress,
                self.now,
            ) {
                Ok(()) => {}
                Err(board::MoveError::Sql(e)) => return Err(Error::Sql(e)),
                Err(e) => return Err(Error::Moved(*n, e.to_string())),
            }
        }
        match w.log(self.slug, &format!("feedback={count}"), &[], self.now) {
            Ok(()) => {}
            Err(board::LogError::Sql(e)) => return Err(Error::Sql(e)),
            Err(_) => return Err(Error::Card),
        }
        tx.commit()?;
        let sync = if owners.is_empty() {
            None
        } else {
            self.sync_draft()?
        };
        Ok((pr, count, owners, sync))
    }
}
