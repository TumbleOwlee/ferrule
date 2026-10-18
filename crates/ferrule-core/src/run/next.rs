//! The next step of a run after an agent's status line, for the gate 2,
//! re-plan, branch-review, gate 4 and CI decisions: logs the events the
//! outcome implies and names the one next action.

use rusqlite::Connection;

use crate::artifacts;
use crate::card::board::{self, State};
use crate::guide::Topic;
use crate::project::Layout;

/// Where in the run the status line comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum At {
    /// Planner, plan reviewer, the spec author's issue answer.
    Gate2,
    /// A re-plan's planner and plan reviewer.
    Replan,
    /// The branch review.
    Gate3,
    /// The spec author's PR body.
    Gate4,
    /// `ci=<exit code of gh pr checks>`.
    Ci,
}

impl At {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            At::Gate2 => "gate2",
            At::Replan => "replan",
            At::Gate3 => "gate3",
            At::Gate4 => "gate4",
            At::Ci => "ci",
        }
    }
}

/// The tokens of a status line the decision reads.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Status {
    pub status: Option<String>,
    pub count: Option<String>,
    pub issue: Option<String>,
    pub ci: Option<String>,
}

impl Status {
    /// Reads `status=`, `count=`, `issue=`, `ci=` from any number of
    /// whitespace-separated words; other tokens are ignored.
    #[must_use]
    pub fn parse(words: &[String]) -> Self {
        let mut s = Status::default();
        for t in words.iter().flat_map(|w| w.split_whitespace()) {
            if let Some((k, v)) = t.split_once('=') {
                let slot = match k {
                    "status" => &mut s.status,
                    "count" => &mut s.count,
                    "issue" => &mut s.issue,
                    "ci" => &mut s.ci,
                    _ => continue,
                };
                *slot = Some(v.to_owned());
            }
        }
        s
    }
}

#[derive(Debug)]
pub enum Error {
    /// The status line lacks what `at` needs.
    Usage,
    Card,
    /// A status `at` does not take.
    Unknown(String),
    Sql(rusqlite::Error),
}

impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Self {
        Error::Sql(e)
    }
}

struct Ctx<'a> {
    conn: &'a mut Connection,
    layout: &'a Layout,
    slug: &'a str,
    at: At,
    now: &'a str,
    out: Vec<String>,
}

impl Ctx<'_> {
    fn log(&mut self, words: &[&str]) -> rusqlite::Result<()> {
        let (event, args) = words.split_first().expect("an event");
        let args: Vec<String> = args.iter().map(|&a| a.to_owned()).collect();
        match board::log(self.conn, self.slug, event, &args, self.now) {
            Ok(()) => self.out.push(format!("logged: {}", words.join(" "))),
            Err(board::LogError::Sql(e)) => return Err(e),
            Err(_) => {}
        }
        Ok(())
    }

    fn next(&mut self, word: &str, reason: Option<&str>) {
        match reason {
            Some(r) => self.out.push(format!("next={word} reason={r}")),
            None => self.out.push(format!("next={word}")),
        }
    }

    fn run(&mut self, command: &str) {
        self.out.push(format!("run: {command}"));
    }

    fn act(&mut self, what: &str) {
        self.out.push(format!("do: {what}"));
    }

    fn guide(&mut self, topic: Topic) {
        self.out.push(format!(
            "guide: ferrule guide {} --slug {}",
            topic.name(),
            self.slug
        ));
    }

    fn field(&self, key: &str) -> rusqlite::Result<Option<String>> {
        board::field(self.conn, self.slug, key)
    }

    fn needs_user(&self) -> bool {
        self.layout
            .artifact(self.slug, "review.verdict.md")
            .is_some_and(|v| artifacts::needs_user(&v))
    }

    /// More than two plan summary rows from s1 on.
    fn outgrown(&self) -> bool {
        self.layout
            .artifact(self.slug, "plan.summary.md")
            .is_some_and(|s| artifacts::stage_rows(&s) > 2)
    }

    fn gate_guide(&mut self) {
        if self.at == At::Replan {
            self.guide(Topic::Replan);
        } else {
            self.guide(Topic::Gate2);
        }
    }

    /// The lowest stage still open, in progress or in review.
    fn open_stage(&self) -> rusqlite::Result<Option<u32>> {
        Ok(board::stages(self.conn, self.slug)?
            .into_iter()
            .find(|(_, _, st)| *st != State::Done)
            .map(|(n, _, _)| n))
    }
}

/// A `replan from=…` log body.
fn is_replan(body: &str) -> bool {
    let mut w = body.split(' ');
    w.next() == Some("replan") && w.next().is_some_and(|t| t.starts_with("from="))
}

fn first_token(body: &str) -> &str {
    body.split(' ').next().unwrap_or("")
}

/// Decides the next step; returns the lines to print.
pub fn next(
    conn: &mut Connection,
    layout: &Layout,
    slug: &str,
    at: At,
    status: &Status,
    now: &str,
) -> Result<Vec<String>, Error> {
    let word = match at {
        At::Ci => status.ci.clone().ok_or(Error::Usage)?,
        _ => status.status.clone().ok_or(Error::Usage)?,
    };
    if board::state(conn, slug)?.is_none() {
        return Err(Error::Card);
    }
    let events = board::events(conn, slug)?;
    let window: Vec<&str> = if at == At::Gate2 {
        events.iter().map(|(_, b)| b.as_str()).collect()
    } else {
        let from = events
            .iter()
            .rposition(|(_, b)| is_replan(b))
            .map_or(0, |i| i + 1);
        events[from..].iter().map(|(_, b)| b.as_str()).collect()
    };
    let plan_reviews: Vec<&str> = window
        .iter()
        .map(|b| first_token(b))
        .filter(|t| t.starts_with("plan-review="))
        .collect();
    let round = plan_reviews.len() + 1;
    let count: u64 = status
        .count
        .as_deref()
        .and_then(|c| c.parse().ok())
        .unwrap_or(0);
    let mut c = Ctx {
        conn,
        layout,
        slug,
        at,
        now,
        out: Vec::new(),
    };
    let mut branch_clean = false;
    match (at, word.as_str()) {
        (At::Gate2 | At::Replan | At::Gate4, "question") => {
            c.next("relay", None);
            c.act("relay the question file to the user and resume the agent with the answers");
            c.guide(Topic::Question);
        }
        (At::Gate2 | At::Replan, "arch-gap") => {
            c.next("arch-gap", None);
            c.act("spawn the architect, then resume the planner with its summary");
            c.gate_guide();
        }
        (At::Gate2 | At::Replan, "split") => {
            c.next("split", None);
            c.act("handle the split request");
            c.guide(Topic::Split);
        }
        (At::Gate2 | At::Replan, "behavior-change") => {
            c.next("behavior-change", None);
            c.act(
                "the plan found a behavior change in a non-behavior run: back to the spec author",
            );
            c.guide(Topic::Principles);
        }
        (At::Gate2 | At::Replan, "ready") => {
            if plan_reviews
                .last()
                .is_some_and(|t| t.starts_with("plan-review=findings"))
            {
                c.next("follow-up", None);
                c.act("spawn the plan reviewer again for a follow-up round");
            } else {
                c.next("review", None);
                c.act("spawn the plan reviewer");
            }
            c.gate_guide();
        }
        (At::Gate2 | At::Replan, "findings" | "clean") => {
            plan_review(&mut c, &word, round, count, &events)?;
        }
        (At::Gate2, "new" | "reuse") => gate2_decision(&mut c, &word, status)?,
        (At::Gate3, "findings") if count > 0 => branch_findings(&mut c, count)?,
        (At::Gate3, "findings" | "clean") => branch_clean = true,
        (At::Gate4, "ready") => {
            c.next("stop", None);
            c.run(&format!("ferrule prompt render {slug} gate4"));
            c.act("show the gate 4 stop and wait for the user's approval");
            c.guide(Topic::Gate4);
        }
        (At::Ci, ci) => ci_result(&mut c, ci)?,
        _ => return Err(Error::Unknown(word)),
    }
    if branch_clean {
        branch_resolved(&mut c, &window)?;
    }
    Ok(c.out)
}

/// A plan review's verdict: logged with its round; a third round, or a
/// clean one, ends the review.
fn plan_review(
    c: &mut Ctx,
    word: &str,
    round: usize,
    count: u64,
    events: &[(String, String)],
) -> rusqlite::Result<()> {
    let round_kv = format!("round={round}");
    if word == "findings" {
        let count_kv = format!("count={count}");
        c.log(&["plan-review=findings", &count_kv, &round_kv])?;
        if round <= 2 {
            c.next("revise", None);
            let dir = format!(".ferrule/runs/{}", c.slug);
            c.run(&format!("cp {dir}/plan.md {dir}/plan.prev.md"));
            c.act("resume the planner with the review file");
            c.gate_guide();
            return Ok(());
        }
    } else {
        c.log(&["plan-review=clean", &round_kv])?;
    }
    if c.at == At::Gate2 {
        ask_issue(c);
        Ok(())
    } else {
        replan_decision(c, events)
    }
}

fn branch_findings(c: &mut Ctx, count: u64) -> rusqlite::Result<()> {
    let open = format!("review={count}-open");
    c.log(&[&open, "scope=branch"])?;
    c.next("fix", None);
    if c.open_stage()?.is_some() {
        c.act("resume the implementer on the findings, then the gauntlet and the same reviewer's follow-up");
        c.guide(Topic::StageReview);
    } else {
        c.act("move the implicated stage to inprogress, resume its implementer, gauntlet, then a fresh branch review");
        c.guide(Topic::Gate3);
    }
    Ok(())
}

fn ci_result(c: &mut Ctx, ci: &str) -> rusqlite::Result<()> {
    let event = format!("ci={ci}");
    c.log(&[&event])?;
    let slug = c.slug;
    if ci == "0" {
        c.next("done", None);
        c.run(&format!("ferrule run cleanup {slug}"));
        c.act("clean up the run");
        c.guide(Topic::Cleanup);
    } else {
        c.next("fix", None);
        let branch = c.field("branch")?.unwrap_or_default();
        c.run(&format!("ferrule pr failed {branch}"));
        c.act("read the failed workflow and fix it");
        c.guide(Topic::Gate4);
    }
    Ok(())
}

fn ask_issue(c: &mut Ctx) {
    c.next("issue", None);
    c.act("ask the spec author for the tracking issue");
    c.guide(Topic::Gate2);
}

/// Gate 2 stops unless a fast-lane run with a dated architecture stop, no
/// Needs user item, a small plan and a new issue.
fn gate2_decision(c: &mut Ctx, word: &str, status: &Status) -> rusqlite::Result<()> {
    let issue = status.issue.as_deref().unwrap_or("new");
    let reason = if c.field("lane")?.as_deref() != Some("fast") {
        Some("lane")
    } else if !c.field("arch")?.is_some_and(|a| {
        let b = a.as_bytes();
        b.len() >= 5 && b[..4].iter().all(u8::is_ascii_digit) && b[4] == b'-'
    }) {
        Some("no-arch-stop")
    } else if c.needs_user() {
        Some("needs-user")
    } else if c.outgrown() {
        Some("outgrown")
    } else if word == "reuse" {
        Some("reuse")
    } else {
        None
    };
    let slug = c.slug;
    if let Some(r) = reason {
        c.next("stop", Some(r));
        c.run(&format!(
            "ferrule prompt render {slug} gate2 --issue {issue}"
        ));
        c.act("show the gate 2 stop and wait for the user's approval");
    } else {
        c.next("notice", None);
        c.run(&format!(
            "ferrule prompt render {slug} gate2-notice --issue new"
        ));
        c.act("show the notice without stopping, then act on approval as if given");
    }
    c.guide(Topic::Gate2);
    Ok(())
}

/// A re-plan asked at gate 4 goes on without stopping when nothing the
/// user approved moved; else it stops.
fn replan_decision(c: &mut Ctx, events: &[(String, String)]) -> rusqlite::Result<()> {
    let mut from = String::new();
    let mut stop_before_replan = String::new();
    let mut last_stop = String::new();
    for (_, body) in events {
        if first_token(body) == "replan" {
            if let Some(f) = body
                .split(' ')
                .filter_map(|t| t.strip_prefix("from="))
                .next_back()
            {
                f.clone_into(&mut from);
            }
            if is_replan(body) {
                stop_before_replan.clone_from(&last_stop);
            }
        }
        if first_token(body) == "stop" {
            last_stop.clone_from(body);
        }
    }
    let ar = c
        .layout
        .artifact(c.slug, "architecture.summary.md")
        .and_then(|a| artifacts::rev(&a));
    let arch_rev: u32 = board::last_token(c.conn, c.slug, "arch-rev")?
        .and_then(|r| r.parse().ok())
        .unwrap_or(0);
    let arch_moved = ar.is_some_and(|ar| ar > arch_rev);
    let reason = if !stop_before_replan.split(' ').any(|t| t == "gate=gate4") {
        Some("not-gate4")
    } else if c.needs_user() {
        Some("needs-user")
    } else if arch_moved {
        Some("arch-moved")
    } else if c.field("lane")?.as_deref() == Some("fast") && c.outgrown() {
        Some("outgrown")
    } else {
        None
    };
    let slug = c.slug;
    if let Some(r) = reason {
        c.next("stop", Some(r));
        c.run(&format!(
            "ferrule prompt render {slug} replan --from {from}"
        ));
        c.act("show the re-plan stop and wait for the user's approval");
    } else {
        c.log(&["replan=pre-authorized"])?;
        c.next("notice", None);
        c.run(&format!(
            "ferrule prompt render {slug} replan-notice --from {from}"
        ));
        c.act("show the notice without stopping, then continue with the re-planned stages");
    }
    c.guide(Topic::Replan);
    Ok(())
}

/// A clean branch review: resolves the findings it followed up, then the
/// next stage's stop decision, gate 3's stop, or the PR.
fn branch_resolved(c: &mut Ctx, window: &[&str]) -> rusqlite::Result<()> {
    let mut first: Option<String> = None;
    for t in window.iter().map(|b| first_token(b)) {
        if t == "review=clean"
            || t.strip_prefix("review=")
                .and_then(|r| r.strip_suffix("-resolved"))
                .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
        {
            first = None;
        } else if first.is_none()
            && let Some(n) = t
                .strip_prefix("review=")
                .and_then(|r| r.strip_suffix("-open"))
                .filter(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
        {
            first = Some(n.to_owned());
        }
    }
    match first {
        Some(n) => c.log(&[&format!("review={n}-resolved")])?,
        None => c.log(&["review=clean"])?,
    }
    let slug = c.slug;
    if let Some(n) = c.open_stage()? {
        c.next("stage-stop", None);
        c.run(&format!("ferrule run stage-stop {slug} s{n} --threads <n>"));
        c.act("run the stage stop decision with the PR thread count");
        c.guide(Topic::StageReview);
    } else if c.needs_user() && c.field("lane")?.as_deref() == Some("full") {
        c.next("stop", None);
        c.run(&format!("ferrule prompt render {slug} gate3"));
        c.act("show the gate 3 stop with the open Needs user item");
        c.guide(Topic::Gate3);
    } else {
        let reason = c.needs_user().then_some("needs-user");
        c.next("pr", reason);
        c.act("spawn the spec author for the PR body");
        c.guide(Topic::Gate4);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ut_status_tokens_from_one_or_many_words() {
        let one = Status::parse(&["status=findings count=2 user=0 file=x".to_owned()]);
        let many = Status::parse(&[
            "status=findings".to_owned(),
            "count=2".to_owned(),
            "stage=[s2]".to_owned(),
        ]);
        assert_eq!(one.status.as_deref(), Some("findings"));
        assert_eq!(one.count, many.count);
        assert_eq!(many.issue, None);
    }
}
