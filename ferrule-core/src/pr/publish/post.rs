//! The issues a run files and the comments it posts: the tracking issue,
//! slice, split-off and follow-up issues, and issue comments.

use std::fmt::Write as _;

use super::{
    Action, Error, ItemRef, Run, Target, attribution, board, body, remote, snapshot, split_title,
};

impl Run<'_> {
    /// Posts `target`; the issue posted to and what happened.
    pub fn file(
        &mut self,
        target: Target,
        reuse: Option<ItemRef>,
    ) -> Result<(ItemRef, Action), Error> {
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
    fn file_follow_up(&mut self, k: u32) -> Result<(ItemRef, Action), Error> {
        let issue = self.issue()?;
        if let Some(n) = self
            .posted("follow-up")?
            .into_iter()
            .find_map(|(n, _, kk)| if kk == Some(k) { n } else { None })
        {
            return Ok((n, Action::Recorded));
        }
        let (title, body) = split_title(&self.artifact(&format!("follow-up-{k}.md"))?);
        let body = format!("{}\n\nFound in {}\n", body.trim_end(), issue.cite());
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
    fn file_comment(&mut self) -> Result<(ItemRef, Action), Error> {
        let issue = self.issue()?;
        let text = self.artifact("issue-comment.md")?;
        let text = attribution::strip(&text).unwrap_or(text);
        let sum = crate::install::hash(&text);
        let mut targets = vec![issue.clone()];
        if self.field("kind")?.as_deref() == Some("slice")
            && let Some(epic) = remote::refs(self.field("epic")?.as_deref())
                .into_iter()
                .next()
        {
            targets.push(epic);
        }
        let done = self.posted("comment")?;
        // The sum rides along hidden, so a comment posted before a crash kept
        // its `posted=` line is found on the issue rather than posted again.
        let marker = format!("<!-- ferrule:comment sum={sum} -->");
        let body = format!("{}\n\n{marker}\n", text.trim_end());
        let mut action = Action::Recorded;
        for t in targets {
            if done
                .iter()
                .any(|(n, s, _)| n.as_ref() == Some(&t) && s.as_deref() == Some(sum.as_str()))
            {
                continue;
            }
            let on_issue = self
                .tracker
                .view(&t)?
                .comments
                .iter()
                .any(|c| c.body.contains(&marker));
            if !on_issue {
                self.tracker.comment(&t, &body)?;
            }
            self.log(
                "posted=comment",
                &[format!("issue={t}"), format!("sum={sum}")],
            )?;
            action = Action::Commented;
        }
        Ok((issue, action))
    }

    fn file_run(&mut self, reuse: Option<ItemRef>) -> Result<(ItemRef, Action), Error> {
        let rendered = snapshot(self.conn, |r| body::issue(r, self.layout, self.slug, None))?;
        let (title, body) = split_title(&rendered);
        let current = remote::refs(self.field("issue")?.as_deref())
            .into_iter()
            .next();
        let (n, action) = match (reuse, current) {
            (Some(n), _) => {
                self.tracker.edit(&n, &title, &body)?;
                (n, Action::Edited)
            }
            (None, Some(n)) if self.field("kind")?.as_deref() == Some("epic") => {
                self.tracker.edit(&n, &title, &body)?;
                (n, Action::Edited)
            }
            (None, Some(n)) => (n, Action::Recorded),
            (None, None) => self.adopt_or_create(&title, &body)?,
        };
        self.set("issue", &n.to_string())?;
        Ok((n, action))
    }

    fn file_slice(&mut self, slice: u32) -> Result<(ItemRef, Action), Error> {
        let Some(at) = (slice as usize).checked_sub(1) else {
            return Err(Error::Body(body::Error::NoSlice(slice)));
        };
        let mut slots = remote::slots(self.field("slices")?.as_deref());
        if let Some(Some(n)) = slots.get(at) {
            return Ok((n.clone(), Action::Recorded));
        }
        let rendered = snapshot(self.conn, |r| {
            body::issue(r, self.layout, self.slug, Some(slice))
        })?;
        let (title, body) = split_title(&rendered);
        let (n, action) = self.adopt_or_create(&title, &body)?;
        if slots.len() <= at {
            slots.resize(at + 1, None);
        }
        slots[at] = Some(n.clone());
        self.set("slices", &remote::slot_list(&slots))?;
        Ok((n, action))
    }

    fn file_split(&mut self) -> Result<(ItemRef, Action), Error> {
        let issue = self.issue()?.cite();
        let (title, body) = split_title(&self.artifact("split-issue.md")?);
        let epic = if self.field("kind")?.as_deref() == Some("slice") {
            remote::refs(self.field("epic")?.as_deref())
                .into_iter()
                .next()
        } else {
            None
        };
        let mut text = String::new();
        if let Some(e) = &epic {
            let _ = write!(
                text,
                "**Part of epic** {} · split from {issue}\n\n",
                e.cite()
            );
        }
        text.push_str(body.trim_end());
        let _ = write!(text, "\n\nSplit from {issue}\n");
        let (n, action) = self.adopt_or_create(&title, &text)?;
        if let (Some(e), Action::Created) = (&epic, action) {
            let note = format!("Split off {} from {issue}.\n", n.cite());
            self.tracker.comment(e, &note)?;
        }
        let mut split = remote::refs(self.field("split")?.as_deref());
        if !split.contains(&n) {
            split.push(n.clone());
            self.set("split", &remote::ref_list(&split))?;
        }
        Ok((n, action))
    }
}
