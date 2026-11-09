//! The issues a run files and the comments it posts: the tracking issue,
//! slice, split-off and follow-up issues, and issue comments.

use std::fmt::Write as _;

use crate::artifacts;
use crate::issue::changelog::keep_changelog;
use crate::remote::{Milestones, NewIssue, SearchState};

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
        let text = self.artifact(&format!("follow-up-{k}.md"))?;
        let (n, action) = if self.tracker.milestones().is_some() {
            self.file_native_follow_up(&text, &issue)?
        } else {
            let (title, body) = split_title(&text);
            let body = format!("{}\n\nFound in {}\n", body.trim_end(), issue.cite());
            self.adopt_or_create(&title, &body)?
        };
        self.log(
            "posted=follow-up",
            &[format!("issue={n}"), format!("k={k}")],
        )?;
        Ok((n, action))
    }

    /// A follow-up on a tracker with milestones: in the milestone its draft
    /// names when the run is a slice run, related to `issue`.
    fn file_native_follow_up(
        &self,
        text: &str,
        issue: &ItemRef,
    ) -> Result<(ItemRef, Action), Error> {
        let d = artifacts::draft(text);
        let ms = self.milestones()?;
        let milestone = match d.milestone.as_deref() {
            Some(title) if self.field("kind")?.as_deref() == Some("slice") => {
                let found = ms
                    .list(SearchState::All)?
                    .into_iter()
                    .find(|m| m.title == title)
                    .ok_or_else(|| Error::Remote(remote::Error::NotFound(title.to_owned())))?;
                if !found.open {
                    return Err(Error::MilestoneClosed(found.number));
                }
                Some((found.number, found.title))
            }
            _ => None,
        };
        let labels = self.draft_labels(d.areas.as_deref());
        let (n, action) = self.file_linked(&d.title, &d.body, &labels, milestone.as_ref())?;
        if !ms.links(&n)?.relates_to.contains(issue) {
            ms.add_relates_to(&n, issue)?;
        }
        Ok((n, action))
    }

    /// The labels of `areas`, or of the brief's when the draft names none.
    fn draft_labels(&self, areas: Option<&[String]>) -> Vec<String> {
        match areas {
            Some(a) if !a.is_empty() => {
                artifacts::area_labels(&self.root.join("docs").join("specs"), a)
            }
            _ => self.run_labels(),
        }
    }

    /// Adopts the open issue titled `title` (inside `milestone` when given),
    /// else files it, labelled; an adopted one is edited only when its text
    /// differs and gets only the labels it lacks.
    fn file_linked(
        &self,
        title: &str,
        body: &str,
        labels: &[String],
        milestone: Option<&(u64, String)>,
    ) -> Result<(ItemRef, Action), Error> {
        let ms = self.milestones()?;
        let found = match milestone {
            Some((m, _)) => ms.find_open_in(*m, title)?,
            None => self.tracker.find_open(title)?,
        };
        if let Some(n) = found {
            self.edit_if_changed(&n, title, body)?;
            self.label_missing(&n, labels)?;
            return Ok((n, Action::Adopted));
        }
        if !labels.is_empty() {
            ms.ensure_labels(labels)?;
        }
        let n = ms.create_issue(&NewIssue {
            title,
            body,
            milestone: milestone.map(|(_, t)| t.as_str()),
            labels,
        })?;
        Ok((n, Action::Created))
    }

    /// issue-comment.md on the tracking issue and a slice's epic, each
    /// target once per text: a `posted=comment issue=<n> sum=<hash>`
    /// marks one done, so a rerun after a failure posts only the rest. On a
    /// tracker with milestones the run's issue alone is commented, then
    /// labelled, and a slice run's kept change is written to its milestone.
    fn file_comment(&mut self) -> Result<(ItemRef, Action), Error> {
        let issue = self.issue()?;
        let text = self.artifact("issue-comment.md")?;
        let text = attribution::strip(&text).unwrap_or(text);
        let sum = crate::install::hash(&text);
        let native = self.tracker.milestones().is_some();
        let mut targets = vec![issue.clone()];
        if !native
            && self.field("kind")?.as_deref() == Some("slice")
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
        if native {
            self.label_missing(&issue, &self.run_labels())?;
            self.write_kept_change()?;
        }
        Ok((issue, action))
    }

    /// The milestone on the card of a slice run.
    fn slice_milestone(&self) -> Result<Option<u64>, Error> {
        if self.field("kind")?.as_deref() != Some("slice") {
            return Ok(None);
        }
        Ok(self
            .field("milestone")?
            .and_then(|n| n.trim().parse::<u64>().ok()))
    }

    /// A slice run's kept change written to its milestone: the description
    /// read again just before, the edits applied and one changelog entry
    /// added. Nothing is written when the entry is there already or an old
    /// text is gone.
    fn write_kept_change(&self) -> Result<(), Error> {
        if self.field("kind")?.as_deref() != Some("slice") {
            return Ok(());
        }
        let Some(m) = self.slice_milestone()? else {
            return Ok(());
        };
        let path = self.layout.run_dir(self.slug).join("milestone-change.md");
        let raw = self.artifact("milestone-change.md")?;
        let change = crate::issue::changelog::parse(&raw).ok_or(Error::Missing(path))?;
        let sum = crate::install::hash(&raw);
        let ms = self.milestones()?;
        let current = ms.get(m)?;
        if crate::issue::changelog::has_entry(&current.description, &sum) {
            return Ok(());
        }
        let login = ms.login()?;
        let entry = crate::issue::changelog::Entry {
            at: self.now,
            author: &login,
            decision: &change.decision,
            summary: &change.summary,
            sum: &sum,
        };
        let description = crate::issue::changelog::apply(&current.description, &change, &entry)
            .map_err(|e| Error::MilestoneStale(e.0))?;
        ms.set_description(m, &description)?;
        Ok(())
    }

    /// Whether this run files its epic as a milestone.
    ///
    /// # Errors
    /// The state database.
    pub fn files_milestone(&self) -> Result<bool, Error> {
        Ok(self.field("kind")?.as_deref() == Some("epic") && self.tracker.milestones().is_some())
    }

    fn milestones(&self) -> Result<&dyn Milestones, Error> {
        self.tracker.milestones().ok_or_else(|| {
            Error::Remote(remote::Error::Unsupported {
                host: self.tracker.name(),
                op: "milestones",
            })
        })
    }

    /// The milestone, the existing issue closed when there is one.
    ///
    /// # Errors
    /// A closed milestone, a host failure, or a missing artifact.
    pub fn file_epic(&mut self, reuse: Option<ItemRef>) -> Result<(u64, Action), Error> {
        let rendered = snapshot(self.conn, |r| {
            body::issue(r, self.layout, self.slug, None, body::Metadata::Native)
        })?;
        let (title, body) = split_title(&rendered);
        let (m, action) = self.settle_milestone(&title, &body)?;
        self.set("milestone", &m.to_string())?;
        let named = remote::refs(self.field("named-issue")?.as_deref())
            .into_iter()
            .next();
        let issue = reuse.or(named);
        if let Some(i) = issue {
            let ms = self.milestones()?;
            if ms.links(&i)?.open {
                let to = self
                    .tracker
                    .view(&i)?
                    .url
                    .as_deref()
                    .and_then(|u| milestone_url(u, m))
                    .unwrap_or_else(|| format!("milestone #{m}"));
                ms.close(&i, &format!("Moved to {to}.\n"))?;
            }
            self.set("issue", &i.to_string())?;
        }
        Ok((m, action))
    }

    /// Finds, adopts or creates the milestone titled `title`, its
    /// description `body` above the changelog it already holds.
    fn settle_milestone(&self, title: &str, body: &str) -> Result<(u64, Action), Error> {
        let ms = self.milestones()?;
        let from_card = self
            .field("milestone")?
            .and_then(|n| n.trim().parse::<u64>().ok());
        let found = if let Some(n) = from_card {
            Some(ms.get(n)?)
        } else {
            ms.list(SearchState::All)?
                .into_iter()
                .find(|m| m.title == title)
        };
        let Some(m) = found else {
            let made = ms.create(title, &keep_changelog(body, ""))?;
            return Ok((made.number, Action::Created));
        };
        if !m.open {
            return Err(Error::MilestoneClosed(m.number));
        }
        let description = keep_changelog(body, &m.description);
        if description == m.description {
            return Ok((m.number, Action::Recorded));
        }
        ms.set_description(m.number, &description)?;
        let action = if from_card.is_some() {
            Action::Edited
        } else {
            Action::Adopted
        };
        Ok((m.number, action))
    }

    fn file_run(&mut self, reuse: Option<ItemRef>) -> Result<(ItemRef, Action), Error> {
        let native = self.tracker.milestones().is_some();
        let metadata = if native {
            body::Metadata::Native
        } else {
            body::Metadata::Prose
        };
        let rendered = snapshot(self.conn, |r| {
            body::issue(r, self.layout, self.slug, None, metadata)
        })?;
        let (title, body) = split_title(&rendered);
        let current = remote::refs(self.field("issue")?.as_deref())
            .into_iter()
            .next();
        let is_epic = self.field("kind")?.as_deref() == Some("epic");
        let (n, action) = match (reuse, current) {
            (Some(n), _) => (self.rewrite(native, &n, &title, &body)?, Action::Edited),
            (None, Some(n)) if is_epic => {
                (self.rewrite(native, &n, &title, &body)?, Action::Edited)
            }
            (None, Some(n)) => (n, Action::Recorded),
            (None, None) if native => self.adopt_or_create_native(&title, &body)?,
            (None, None) => self.adopt_or_create(&title, &body)?,
        };
        self.set("issue", &n.to_string())?;
        Ok((n, action))
    }

    /// The issue `n` given the text, an unchanged one left alone on a
    /// tracker with milestones.
    fn rewrite(
        &self,
        native: bool,
        n: &ItemRef,
        title: &str,
        body: &str,
    ) -> Result<ItemRef, Error> {
        if native {
            self.edit_if_changed(n, title, body)?;
            self.label_missing(n, &self.run_labels())?;
        } else {
            self.tracker.edit(n, title, body)?;
        }
        Ok(n.clone())
    }

    fn adopt_or_create_native(&self, title: &str, body: &str) -> Result<(ItemRef, Action), Error> {
        let labels = self.run_labels();
        if let Some(n) = self.tracker.find_open(title)? {
            self.edit_if_changed(&n, title, body)?;
            self.label_missing(&n, &labels)?;
            return Ok((n, Action::Adopted));
        }
        let n = self.tracker.create(title, body)?;
        self.label_missing(&n, &labels)?;
        Ok((n, Action::Created))
    }

    /// The labels of the specs areas the brief names.
    fn run_labels(&self) -> Vec<String> {
        let brief = std::fs::read_to_string(self.layout.run_dir(self.slug).join("spec-diff.md"))
            .unwrap_or_default();
        artifacts::area_labels(
            &self.root.join("docs").join("specs"),
            &artifacts::brief_areas(&brief),
        )
    }

    /// Edits `n` only when its body or its title differs from the text.
    fn edit_if_changed(&self, n: &ItemRef, title: &str, body: &str) -> Result<(), Error> {
        let item = self.tracker.view(n)?;
        if item.body.trim_end() != body.trim_end() || !item.head.contains(&format!(" {title} [")) {
            self.tracker.edit(n, title, body)?;
        }
        Ok(())
    }

    /// Adds to `n` the `labels` it lacks, creating those the repository lacks.
    fn label_missing(&self, n: &ItemRef, labels: &[String]) -> Result<(), Error> {
        if labels.is_empty() {
            return Ok(());
        }
        let ms = self.milestones()?;
        let have = ms.links(n)?.labels;
        let missing: Vec<String> = labels
            .iter()
            .filter(|l| !have.contains(l))
            .cloned()
            .collect();
        if missing.is_empty() {
            return Ok(());
        }
        ms.ensure_labels(&missing)?;
        ms.add_labels(n, &missing)?;
        Ok(())
    }

    fn file_slice(&mut self, slice: u32) -> Result<(ItemRef, Action), Error> {
        let Some(at) = (slice as usize).checked_sub(1) else {
            return Err(Error::Body(body::Error::NoSlice(slice)));
        };
        let mut slots = remote::slots(self.field("slices")?.as_deref());
        if let Some(Some(n)) = slots.get(at) {
            return Ok((n.clone(), Action::Recorded));
        }
        let native = self.tracker.milestones().is_some();
        let metadata = if native {
            body::Metadata::Native
        } else {
            body::Metadata::Prose
        };
        let rendered = snapshot(self.conn, |r| {
            body::issue(r, self.layout, self.slug, Some(slice), metadata)
        })?;
        let (title, body) = split_title(&rendered);
        let (n, action) = if native {
            self.file_slice_in_milestone(slice, &slots, &title, &body)?
        } else {
            self.adopt_or_create(&title, &body)?
        };
        if slots.len() <= at {
            slots.resize(at + 1, None);
        }
        slots[at] = Some(n.clone());
        self.set("slices", &remote::slot_list(&slots))?;
        Ok((n, action))
    }

    /// Slice `slice`'s issue inside the epic's milestone, labelled and
    /// blocked by the slices it depends on; the slot is the caller's to
    /// record once this has returned.
    fn file_slice_in_milestone(
        &self,
        slice: u32,
        slots: &[Option<ItemRef>],
        title: &str,
        body: &str,
    ) -> Result<(ItemRef, Action), Error> {
        let m: u64 = self
            .field("milestone")?
            .and_then(|n| n.trim().parse().ok())
            .ok_or(Error::NoIssue)?;
        let all = artifacts::slices(&self.artifact("slices.md")?);
        let this = all.iter().find(|s| s.n == slice);
        let areas: Vec<String> = this
            .map(|s| {
                s.get("Areas")
                    .split(',')
                    .map(str::trim)
                    .filter(|a| !a.is_empty())
                    .map(str::to_owned)
                    .collect()
            })
            .filter(|a: &Vec<String>| !a.is_empty())
            .unwrap_or_else(|| {
                let brief =
                    std::fs::read_to_string(self.layout.run_dir(self.slug).join("spec-diff.md"))
                        .unwrap_or_default();
                artifacts::brief_areas(&brief)
            });
        let labels = artifacts::area_labels(&self.root.join("docs").join("specs"), &areas);
        let blockers = Self::blockers(this, slots)?;
        let ms = self.milestones()?;
        let (n, action) = if let Some(n) = ms.find_open_in(m, title)? {
            self.edit_if_changed(&n, title, body)?;
            self.label_missing(&n, &labels)?;
            (n, Action::Adopted)
        } else {
            let milestone = ms.get(m)?;
            if !labels.is_empty() {
                ms.ensure_labels(&labels)?;
            }
            let n = ms.create_issue(&NewIssue {
                title,
                body,
                milestone: Some(&milestone.title),
                labels: &labels,
            })?;
            (n, Action::Created)
        };
        let have = ms.links(&n)?.blocked_by;
        for b in blockers {
            if !have.iter().any(|h| h.reference == b) {
                ms.add_blocked_by(&n, &b)?;
            }
        }
        Ok((n, action))
    }

    /// The issues of the slices `this` depends on; an unfiled one is
    /// `NoSlice`.
    fn blockers(
        this: Option<&artifacts::Slice>,
        slots: &[Option<ItemRef>],
    ) -> Result<Vec<ItemRef>, Error> {
        let mut out = Vec::new();
        for d in this
            .map(|s| s.get("Depends on"))
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|d| !d.is_empty() && *d != "none")
        {
            let n: u32 = d
                .parse()
                .map_err(|_| Error::Body(body::Error::NoSlice(0)))?;
            let at = (n as usize).checked_sub(1);
            match at.and_then(|i| slots.get(i)).cloned().flatten() {
                Some(r) => out.push(r),
                None => return Err(Error::Body(body::Error::NoSlice(n))),
            }
        }
        Ok(out)
    }

    fn file_split(&mut self) -> Result<(ItemRef, Action), Error> {
        let (n, action) = if self.tracker.milestones().is_some() {
            self.file_native_split()?
        } else {
            self.file_prose_split()?
        };
        let mut split = remote::refs(self.field("split")?.as_deref());
        if !split.contains(&n) {
            split.push(n.clone());
            self.set("split", &remote::ref_list(&split))?;
        }
        Ok((n, action))
    }

    /// A split-off on a tracker with milestones: in the slice's milestone
    /// for a slice run, a child of the issue it came from unless it has
    /// another parent, and noted in the milestone's changelog.
    fn file_native_split(&self) -> Result<(ItemRef, Action), Error> {
        let source = self.issue()?;
        let d = artifacts::draft(&self.artifact("split-issue.md")?);
        let ms = self.milestones()?;
        let in_milestone = match self.slice_milestone()? {
            Some(m) => Some((m, ms.get(m)?.title)),
            None => None,
        };
        let labels = self.draft_labels(d.areas.as_deref());
        let (n, action) = self.file_linked(&d.title, &d.body, &labels, in_milestone.as_ref())?;
        if ms.links(&n)?.parent.is_none() {
            ms.set_parent(&n, &source)?;
        }
        if let Some((m, _)) = in_milestone {
            self.note_split(m, &n, &d.title)?;
        }
        Ok((n, action))
    }

    /// One changelog entry on milestone `m` for split-off `n`, once.
    fn note_split(&self, m: u64, n: &ItemRef, title: &str) -> Result<(), Error> {
        let ms = self.milestones()?;
        let sum = crate::install::hash(&format!("split:{n}"));
        let current = ms.get(m)?;
        if crate::issue::changelog::has_entry(&current.description, &sum) {
            return Ok(());
        }
        let login = ms.login()?;
        let decision = format!("Split off {}", n.cite());
        let entry = crate::issue::changelog::Entry {
            at: self.now,
            author: &login,
            decision: &decision,
            summary: title,
            sum: &sum,
        };
        let description = crate::issue::changelog::append(&current.description, &entry);
        ms.set_description(m, &description)?;
        Ok(())
    }

    fn file_prose_split(&self) -> Result<(ItemRef, Action), Error> {
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
        Ok((n, action))
    }
}

/// `https://host/o/r/issues/12` → `https://host/o/r/milestone/<m>`.
fn milestone_url(issue_url: &str, m: u64) -> Option<String> {
    let at = issue_url.rfind("/issues/")?;
    Some(format!("{}/milestone/{m}", &issue_url[..at]))
}
