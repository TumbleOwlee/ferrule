//! The draft PR: opened or adopted once, its body kept in step with the
//! board.

use super::{
    Error, Form, ItemRef, Name, PrState, Run, Sync, Typed, board, body, numbers, snapshot,
    split_title,
};

impl Run<'_> {
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
