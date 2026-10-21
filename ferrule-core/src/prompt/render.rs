//! A stop's whole message, rendered from the run's files and cards: the
//! approval prompt, the question batch or the notice line. The
//! orchestrator relays it verbatim and composes nothing itself. Approval
//! and question stops are logged on the parent card.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use rusqlite::Connection;

use crate::artifacts;
use crate::card::board::{self, State};
use crate::card::typed::{Grant, Name, Typed, Verdict};
use crate::project::Layout;
use crate::prompt::{approval, lines};
use crate::store::Board;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Gate1,
    Arch,
    Epic,
    Gate2,
    Stage,
    Replan,
    Gate3,
    Gate4,
    Question,
    Blocked,
    Delta,
    Auto,
    Gate2Notice,
    ReplanNotice,
    Approved,
}

impl Kind {
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "gate1" => Kind::Gate1,
            "arch" => Kind::Arch,
            "epic" => Kind::Epic,
            "gate2" => Kind::Gate2,
            "stage" => Kind::Stage,
            "replan" => Kind::Replan,
            "gate3" => Kind::Gate3,
            "gate4" => Kind::Gate4,
            "question" => Kind::Question,
            "blocked" => Kind::Blocked,
            "delta" => Kind::Delta,
            "auto" => Kind::Auto,
            "gate2-notice" => Kind::Gate2Notice,
            "replan-notice" => Kind::ReplanNotice,
            "approved" => Kind::Approved,
            _ => return None,
        })
    }

    /// The `gate=` an approval stop logs.
    fn gate(self) -> &'static str {
        match self {
            Kind::Gate1 => "gate1",
            Kind::Arch => "arch",
            Kind::Epic => "epic",
            Kind::Gate2 => "gate2",
            Kind::Replan => "replan",
            Kind::Gate3 => "gate3",
            _ => "gate4",
        }
    }
}

/// What the viewer made of the files a stop shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Viewed {
    Opened,
    Failed,
    /// No viewer is configured; the paths to open by hand.
    Unavailable(Vec<PathBuf>),
}

#[derive(Debug)]
pub enum Error {
    Usage,
    Card,
    /// The viewer step refused (a summary over its cap): its exit code.
    Show(u8),
    Sql(rusqlite::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Usage => f.write_str("the arguments lack what this prompt needs"),
            Error::Card => f.write_str("the run has no card"),
            Error::Show(code) => write!(f, "the viewer step refused (exit {code})"),
            Error::Sql(e) => write!(f, "state database: {e}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Sql(e) => Some(e),
            Error::Usage | Error::Card | Error::Show(_) => None,
        }
    }
}

impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Self {
        Error::Sql(e)
    }
}

/// The value of the last `<key>=` argument.
fn arg<'a>(args: &'a [(String, String)], key: &str) -> Option<&'a str> {
    args.iter()
        .rev()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

fn rev_of(path: &Path) -> Option<u32> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| artifacts::rev(&t))
}

/// The prompt's fixed block order, by a quoted line's head.
fn rank(l: &str) -> u8 {
    const HEADS: &[(&str, u8)] = &[
        ("Goal: ", 1),
        ("Scenarios: ", 2),
        ("Entries:", 3),
        ("Deferred: ", 4),
        ("Size: ", 5),
        ("Architecture: ", 6),
        ("Interfaces: ", 7),
        ("Surface:", 8),
        ("Slices:", 9),
        ("Plan: ", 10),
        ("Spec since arch: ", 11),
        ("Stops:", 12),
        ("Issue: ", 13),
        ("Split: ", 14),
        ("PR: ", 15),
        ("Follow-up ", 16),
        ("Verification:", 17),
        ("Confirm", 18),
        ("Review: ", 19),
        ("Not checked: ", 20),
        ("Spec changes: ", 22),
        ("Spec: ", 21),
        ("Minors ", 23),
        ("Needs user", 24),
    ];
    HEADS
        .iter()
        .find(|(h, _)| l.starts_with(h))
        .map_or(25, |(_, r)| *r)
}

/// The quoted lines reordered into the prompt's block order; list items
/// keep their header, follow-ups gather under one `Follow-ups:`.
fn reorder(lines: Vec<String>, issue: Option<&str>) -> Vec<String> {
    let mut keyed: Vec<((u8, u32, u32), String)> = Vec::new();
    let (mut r, mut b, mut n) = (0u8, 0u32, 0u32);
    let mut follow_ups = false;
    for line in lines {
        if line.starts_with("- ") || line.starts_with("  ") {
            n += 1;
            keyed.push(((r, b, n), line));
            continue;
        }
        r = rank(&line);
        b += 1;
        if let Some(t) = line.strip_prefix("Issue: ") {
            let how = match issue {
                None | Some("" | "new") => "new".to_owned(),
                Some(i) => format!("reuse #{i}"),
            };
            n += 1;
            keyed.push(((r, b, n), format!("Issue: {how} — {t}")));
        } else if let Some(t) = line.strip_prefix("Follow-up ") {
            if !follow_ups {
                keyed.push(((r, 0, 0), "Follow-ups:".to_owned()));
                follow_ups = true;
            }
            n += 1;
            keyed.push(((r, b, n), format!("- {t}")));
        } else {
            n += 1;
            keyed.push(((r, b, n), line));
        }
    }
    keyed.sort_by_key(|(k, _)| *k);
    keyed.into_iter().map(|(_, l)| l).collect()
}

/// Only the `Confirm` block of quoted lines.
fn confirm_only(lines: &[String]) -> Vec<String> {
    let mut on = false;
    let mut out = Vec::new();
    for l in lines {
        if l.starts_with("Confirm") {
            on = true;
            out.push(l.clone());
        } else if on && l.starts_with("- ") {
            out.push(l.clone());
        } else {
            on = false;
        }
    }
    out
}

struct Run<'a> {
    conn: &'a mut Connection,
    layout: &'a Layout,
    slug: &'a str,
    now: &'a str,
}

impl Run<'_> {
    fn art(&self, name: &str) -> PathBuf {
        self.layout.run_dir(self.slug).join(name)
    }

    fn field(&self, key: &str) -> rusqlite::Result<Option<String>> {
        board::field(self.conn, self.slug, key)
    }

    /// The value of the parent's newest `name` line.
    fn value(&self, name: Name) -> rusqlite::Result<Option<String>> {
        board::last_value(self.conn, self.slug, name)
    }

    fn log(&mut self, event: &str, args: &[&str]) -> rusqlite::Result<()> {
        let args: Vec<String> = args.iter().map(|&a| a.to_owned()).collect();
        match board::log(self.conn, self.slug, event, &args, self.now) {
            Err(board::LogError::Sql(e)) => Err(e),
            _ => Ok(()),
        }
    }

    fn needs_user(&self) -> bool {
        std::fs::read_to_string(self.art("review.verdict.md"))
            .is_ok_and(|v| artifacts::needs_user(&v))
    }

    /// A `replan=pre-authorized` line newer than the last gate 4 stop.
    fn replan_unseen(&self) -> rusqlite::Result<bool> {
        let events = board::events(self.conn, self.slug, &[Name::ReplanApproved, Name::Stop])?;
        let replan = events
            .iter()
            .rposition(|e| e.typed() == Typed::ReplanApproved(Grant::PreAuthorized));
        let gate4 = events.iter().rposition(|e| {
            e.typed()
                == Typed::Stop {
                    kind: Some("approval"),
                    gate: Some("gate4"),
                }
        });
        Ok(match (replan, gate4) {
            (Some(r), Some(g)) => r > g,
            (Some(_), None) => true,
            _ => false,
        })
    }

    fn stages(&self) -> rusqlite::Result<Vec<(u32, String, State)>> {
        board::stages(self.conn, self.slug)
    }
}

/// The run's state a stop's text depends on.
struct Facts {
    lane: String,
    /// `gate1:` carries no date yet while a spec-diff.md exists.
    brief_pending: bool,
    /// spec-diff.md is shown: the brief is pending or its rev moved past
    /// the last `spec-rev=`.
    want_spec: bool,
    /// decisions.md's line count at the last approval.
    seen: usize,
    stages: usize,
    first: Option<String>,
    decide: String,
}

fn number<T: std::str::FromStr + Default>(v: Option<String>) -> T {
    v.and_then(|v| v.parse().ok()).unwrap_or_default()
}

fn is_stage_row(l: &str) -> bool {
    l.strip_prefix("| s")
        .is_some_and(|r| r.starts_with(|c: char| c.is_ascii_digit()))
}

/// The run's `follow-up-<k>.md` files, sorted.
fn follow_ups(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|d| {
            d.filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| {
                    p.file_stem()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.starts_with("follow-up-"))
                        && p.extension().is_some_and(|e| e == "md")
                })
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

fn issue_word(args: &[(String, String)]) -> String {
    match arg(args, "issue") {
        None | Some("" | "new") => "new".to_owned(),
        Some(i) => format!("reuse #{i}"),
    }
}

impl Run<'_> {
    fn facts(&self) -> rusqlite::Result<Facts> {
        let spec = self.art("spec-diff.md");
        let brief_pending = !board::is_date(self.field("gate1")?.as_deref()) && spec.is_file();
        let since: u32 = if brief_pending {
            0
        } else {
            number(self.value(Name::SpecRev)?)
        };
        let summary = std::fs::read_to_string(self.art("plan.summary.md")).unwrap_or_default();
        let rows: Vec<&str> = summary.lines().filter(|l| is_stage_row(l)).collect();
        Ok(Facts {
            lane: self.field("lane")?.unwrap_or_default(),
            brief_pending,
            want_spec: spec.is_file()
                && (brief_pending || rev_of(&spec).is_some_and(|r| r > since)),
            seen: number(self.value(Name::DecisionsSeen)?),
            stages: rows.len(),
            first: rows
                .first()
                .and_then(|l| l.split('|').nth(1))
                .map(|s| s.replace(' ', "")),
            decide: summary
                .lines()
                .find_map(|l| l.strip_prefix("Decide: "))
                .unwrap_or("")
                .to_owned(),
        })
    }

    /// The files a stop shows in the viewer and those it only quotes.
    fn files(&self, kind: Kind, want_spec: bool) -> rusqlite::Result<(Vec<PathBuf>, Vec<PathBuf>)> {
        let mut show: Vec<PathBuf> = Vec::new();
        let mut quote: Vec<PathBuf> = Vec::new();
        let add = |list: &mut Vec<PathBuf>, name: &str| {
            let p = self.art(name);
            if p.is_file() {
                list.push(p);
            }
        };
        if matches!(
            kind,
            Kind::Arch
                | Kind::Epic
                | Kind::Gate2
                | Kind::Gate2Notice
                | Kind::Replan
                | Kind::ReplanNotice
        ) {
            add(&mut quote, "decisions.md");
        }
        if want_spec
            && matches!(
                kind,
                Kind::Arch | Kind::Epic | Kind::Gate2 | Kind::Gate2Notice | Kind::Replan
            )
        {
            add(&mut show, "spec-diff.md");
        }
        match kind {
            Kind::Gate1 => {
                add(&mut show, "spec-diff.md");
                add(&mut quote, "decisions.md");
            }
            Kind::Arch | Kind::Epic => {
                add(&mut show, "architecture.summary.md");
                add(&mut show, "architecture.md");
                if kind == Kind::Epic {
                    add(&mut show, "slices.md");
                    add(&mut quote, "issue.md");
                }
            }
            Kind::Gate2 | Kind::Gate2Notice | Kind::Replan => {
                if kind == Kind::Replan {
                    let arch_rev: u32 = number(self.value(Name::ArchRev)?);
                    if rev_of(&self.art("architecture.summary.md")).is_some_and(|r| r > arch_rev) {
                        add(&mut show, "architecture.summary.md");
                    }
                }
                add(&mut show, "plan.summary.md");
                if self.needs_user() {
                    add(&mut show, "review.verdict.md");
                }
                if kind != Kind::Replan {
                    add(&mut quote, "issue.md");
                    add(&mut quote, "split-issue.md");
                }
            }
            Kind::Stage | Kind::Gate3 => add(&mut show, "review.verdict.md"),
            Kind::Gate4 => {
                add(&mut show, "pr.md");
                show.extend(follow_ups(&self.layout.run_dir(self.slug)));
                add(&mut show, "review.verdict.md");
                if self.replan_unseen()? {
                    add(&mut show, "plan.summary.md");
                }
            }
            Kind::ReplanNotice => add(&mut show, "plan.summary.md"),
            Kind::Question => add(&mut quote, "question.md"),
            Kind::Blocked | Kind::Delta | Kind::Auto | Kind::Approved => {}
        }
        Ok((show, quote))
    }
}

/// The note under the header when the viewer did not open the files.
fn viewer_note(
    view: &mut dyn FnMut(&[PathBuf]) -> Result<Viewed, u8>,
    show: &[PathBuf],
) -> Result<Option<String>, Error> {
    if show.is_empty() {
        return Ok(None);
    }
    match view(show) {
        Ok(Viewed::Opened) => Ok(None),
        Ok(Viewed::Failed) => Ok(Some("Viewer: failed — open the files manually".to_owned())),
        Ok(Viewed::Unavailable(paths)) => {
            let paths: Vec<String> = paths.iter().map(|p| p.display().to_string()).collect();
            Ok(Some(format!(
                "Viewer: none available — open manually: {}",
                paths.join(" ")
            )))
        }
        Err(code) => Err(Error::Show(code)),
    }
}

/// The quoted lines of every file, plus gate 1's size line, in block order.
fn quoted(
    show: &[PathBuf],
    quote: &[PathBuf],
    seen: usize,
    kind: Kind,
    args: &[(String, String)],
) -> Vec<String> {
    let files: Vec<&Path> = show.iter().chain(quote).map(PathBuf::as_path).collect();
    let mut lines = if files.is_empty() {
        Vec::new()
    } else {
        lines::lines(&files, seen).0
    };
    if kind == Kind::Gate1 && arg(args, "size") == Some("epic") {
        lines.push(format!(
            "Size: epic recommended — {}",
            arg(args, "reason").unwrap_or("")
        ));
    }
    reorder(lines, arg(args, "issue"))
}

/// Renders the stop; `view` opens the files it shows and records them as
/// the approval marker.
pub fn render(
    db: &mut Board,
    layout: &Layout,
    kind: Kind,
    slug: &str,
    args: &[(String, String)],
    now: &str,
    view: &mut dyn FnMut(&[PathBuf]) -> Result<Viewed, u8>,
) -> Result<Vec<String>, Error> {
    render_in(db.conn_mut(), layout, kind, slug, args, now, view)
}

/// [`render`] over a connection core code already holds.
pub(crate) fn render_in(
    conn: &mut Connection,
    layout: &Layout,
    kind: Kind,
    slug: &str,
    args: &[(String, String)],
    now: &str,
    view: &mut dyn FnMut(&[PathBuf]) -> Result<Viewed, u8>,
) -> Result<Vec<String>, Error> {
    if board::state(conn, slug)?.is_none() {
        return Err(Error::Card);
    }
    let mut run = Run {
        conn,
        layout,
        slug,
        now,
    };
    match kind {
        Kind::Approved => return approved(&mut run),
        Kind::Auto => return auto(&run, args),
        Kind::Question => return question(&mut run, args),
        Kind::Blocked => return blocked(&mut run, args),
        Kind::Delta => return delta(&mut run, args),
        _ => {}
    }
    let facts = run.facts()?;
    let (show, quote) = run.files(kind, facts.want_spec)?;
    let first = facts.first.as_deref().unwrap_or("s1");
    match kind {
        Kind::Gate2Notice => {
            let mut out = vec![format!(
                "Gate 2 · {slug} · {} · issue {} · starting {first} (pre-authorized — say stop to hold)",
                facts.decide,
                issue_word(args)
            )];
            out.extend(confirm_only(&quoted(&show, &quote, facts.seen, kind, args)));
            return Ok(out);
        }
        Kind::ReplanNotice => {
            let from = arg(args, "from").ok_or(Error::Usage)?;
            let mut out = vec![format!(
                "Re-plan from {from} · {slug} · {} · starting {from} (pre-authorized at gate 4 — say stop to hold)",
                facts.decide
            )];
            out.extend(confirm_only(&quoted(&show, &quote, facts.seen, kind, args)));
            return Ok(out);
        }
        _ => {}
    }
    let note = viewer_note(view, &show)?;
    let approval = approval_text(&run, kind, args, &facts)?;
    if kind == Kind::Stage {
        run.log("stop", &["kind=stage"])?;
    } else {
        let gate = format!("gate={}", kind.gate());
        run.log("stop", &["kind=approval", &gate])?;
    }
    let mut out = vec![header(
        &approval.title,
        slug,
        &show,
        &facts.lane,
        kind,
        args,
    )];
    out.extend(note);
    out.extend(quoted(&show, &quote, facts.seen, kind, args));
    out.push("Approving will:".to_owned());
    out.extend(approval.will);
    if !approval.options.is_empty() {
        out.push("Options:".to_owned());
        out.extend(approval.options);
    }
    out.push(format!("Answer: {}", approval.answer));
    Ok(out)
}

/// `<title> · <slug>[ · <files with rev>][ · <lane> lane][ · stop: …]`.
fn header(
    title: &str,
    slug: &str,
    show: &[PathBuf],
    lane: &str,
    kind: Kind,
    args: &[(String, String)],
) -> String {
    let mut h = format!("{title} · {slug}");
    if !show.is_empty() {
        let files: Vec<String> = show
            .iter()
            .map(|p| {
                let name = p
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .to_owned();
                match rev_of(p) {
                    Some(r) => format!("{name} rev {r}"),
                    None => name,
                }
            })
            .collect();
        let _ = write!(h, " · {}", files.join(", "));
    }
    if !lane.is_empty() && lane != "full" {
        let _ = write!(h, " · {lane} lane");
    }
    if kind == Kind::Stage {
        let _ = write!(h, " · stop: {}", arg(args, "reason").unwrap_or(""));
        if let Some(d) = arg(args, "detail").filter(|d| !d.is_empty()) {
            let _ = write!(h, " ({d})");
        }
    }
    h
}

/// question.md verbatim under the question header.
fn question(run: &mut Run, args: &[(String, String)]) -> Result<Vec<String>, Error> {
    let q = std::fs::read_to_string(run.art("question.md")).unwrap_or_default();
    let n = q
        .lines()
        .filter(|l| {
            l.strip_prefix("## ").is_some_and(|r| {
                let d = r.bytes().take_while(u8::is_ascii_digit).count();
                r[d..].starts_with(". ")
            })
        })
        .count();
    run.log("stop", &["kind=question"])?;
    let mut out = vec![format!(
        "Question · {} · {} · {} · {n} decision{}",
        run.slug,
        arg(args, "where").unwrap_or("-"),
        arg(args, "agent").unwrap_or("-"),
        plural(n)
    )];
    for (i, l) in q.lines().enumerate() {
        if !(i == 0 && l.starts_with("# ")) {
            out.push(l.to_owned());
        }
    }
    out.push("Answer: <1: a · 2: b · …> · other <text>".to_owned());
    Ok(out)
}

fn blocked(run: &mut Run, args: &[(String, String)]) -> Result<Vec<String>, Error> {
    let sid = arg(args, "stage").unwrap_or("");
    let reason = arg(args, "reason").unwrap_or("");
    run.log("stop", &["kind=blocked"])?;
    Ok(vec![
        format!(
            "Question · {} · {sid} · fe-implementer · 1 decision",
            run.slug
        ),
        format!("## 1. {sid} is blocked: {reason}"),
        "Options:".to_owned(),
        "- a) answer: <text> — relayed to the implementer".to_owned(),
        "- b) re-plan — a fresh planner rewrites the stages not yet committed".to_owned(),
        "- c) stop — the run halts, the board stays for a resume".to_owned(),
        "Answer: <1: a <text> | b | c>".to_owned(),
    ])
}

/// The verdict's Needs user items, one decision each.
fn delta(run: &mut Run, args: &[(String, String)]) -> Result<Vec<String>, Error> {
    let sid = arg(args, "stage").unwrap_or("");
    run.log("stop", &["kind=approval", "gate=delta"])?;
    let verdict = std::fs::read_to_string(run.art("review.verdict.md")).unwrap_or_default();
    let mut items = Vec::new();
    let mut on = false;
    let mut n = 0;
    for l in verdict.lines() {
        if l.starts_with("Needs user") {
            on = true;
        } else if on && let Some(t) = l.strip_prefix("- ") {
            if l == "- none" {
                continue;
            }
            n += 1;
            items.push(format!("## {n}. {t}"));
            items.push("Options:".to_owned());
        } else if on && let Some(t) = l.strip_prefix("  - ") {
            items.push(format!("- {t}"));
        } else {
            on = false;
        }
    }
    let mut out = vec![format!(
        "Question · {} · delta {sid} · fe-reviewer · {n} decision{}",
        run.slug,
        plural(n)
    )];
    if items.is_empty() {
        out.push(String::new());
    }
    out.extend(items);
    out.push(
        "Answer: <1: a · 2: b · …> · other <text> — keep, revert or split each change".to_owned(),
    );
    Ok(out)
}

struct Approval {
    title: String,
    will: Vec<String>,
    options: Vec<String>,
    answer: String,
}

impl Approval {
    /// The answer line follows from whether there are options.
    fn new(title: impl Into<String>, will: Vec<String>, options: Vec<String>) -> Self {
        let answer = if options.is_empty() {
            "approve · change <what> · stop"
        } else {
            "approve [<n>[: <value>] …] · change <what> · stop"
        };
        Approval {
            title: title.into(),
            will,
            options,
            answer: answer.to_owned(),
        }
    }
}

fn strings(v: &[&str]) -> Vec<String> {
    v.iter().map(|&x| x.to_owned()).collect()
}

fn other_lane(lane: &str) -> &'static str {
    if lane == "full" { "fast" } else { "full" }
}

/// Gate 2 and re-plan offer extra stops in the full lane, else a switch.
fn lane_options(lane: &str, example: &str) -> Vec<String> {
    if lane == "full" {
        vec![
            format!("1. also stop after: {example}"),
            "2. switch to the fast lane".to_owned(),
        ]
    } else {
        vec![format!("1. switch to the {} lane", other_lane(lane))]
    }
}

/// `- approve the brief` while the brief is pending.
fn brief(facts: &Facts) -> Vec<String> {
    if facts.brief_pending {
        vec!["- approve the brief".to_owned()]
    } else {
        Vec::new()
    }
}

fn approval_text(
    run: &Run,
    kind: Kind,
    args: &[(String, String)],
    facts: &Facts,
) -> Result<Approval, Error> {
    Ok(match kind {
        Kind::Gate1 => gate1(args),
        Kind::Arch => arch(facts),
        Kind::Epic => epic(run, args, facts),
        Kind::Gate2 => gate2(run, args, facts)?,
        Kind::Stage => {
            let sid = arg(args, "stage").unwrap_or("");
            Approval::new(
                format!("Stage {sid}"),
                vec![
                    format!("- commit {sid}, push, update the draft PR"),
                    "- spawn the next stage".to_owned(),
                ],
                Vec::new(),
            )
        }
        Kind::Replan => {
            let from = arg(args, "from").unwrap_or("");
            Approval::new(
                format!("Re-plan from {from}"),
                vec![
                    format!("- replace the cards of {from} onward"),
                    "- update the draft PR".to_owned(),
                    format!("- spawn a fresh implementer at {from}"),
                ],
                lane_options(&facts.lane, "<stages>"),
            )
        }
        Kind::Gate3 => Approval::new(
            "Gate 3",
            strings(&["- hand the branch to fe-author for the PR body (gate 4)"]),
            Vec::new(),
        ),
        Kind::Gate4 => gate4(run, args)?,
        _ => return Err(Error::Usage),
    })
}

fn gate1(args: &[(String, String)]) -> Approval {
    if arg(args, "size") == Some("epic") {
        return Approval::new(
            "Gate 1",
            strings(&[
                "- record the run as an epic (option 1) or a single run in the lane chosen",
                "- hand the brief to the architect",
            ]),
            strings(&[
                "1. epic — a shared skeleton, one issue per slice (recommended)",
                "2. single run, full lane",
                "3. single run, fast lane",
            ]),
        );
    }
    let rec = arg(args, "recommend")
        .filter(|r| !r.is_empty())
        .unwrap_or("full");
    let opts = ["full", "fast"]
        .iter()
        .enumerate()
        .map(|(i, l)| {
            let tag = if *l == rec { " (recommended)" } else { "" };
            format!("{}. {l} lane{tag}", i + 1)
        })
        .collect();
    Approval::new(
        "Gate 1",
        strings(&["- settle the lane", "- hand the brief to the architect"]),
        opts,
    )
}

fn arch(facts: &Facts) -> Approval {
    let (will, opts) = match facts.lane.as_str() {
        "fast" => {
            let mut w = brief(facts);
            w.extend(strings(&[
                "- hand brief and architecture to the planner",
                "- run gate 2 without a stop once the plan review is clean",
                "- next stop: gate 4",
            ]));
            (w, strings(&["1. switch to the full lane"]))
        }
        "" => (
            strings(&["- hand the architecture to the planner"]),
            strings(&["1. full lane", "2. fast lane"]),
        ),
        _ => (
            strings(&["- hand brief and architecture to the planner"]),
            Vec::new(),
        ),
    };
    Approval::new("Architecture", will, opts)
}

fn epic(run: &Run, args: &[(String, String)], facts: &Facts) -> Approval {
    let slices = std::fs::read_to_string(run.art("slices.md")).unwrap_or_default();
    let n = slices
        .lines()
        .filter(|l| {
            l.strip_prefix("## Slice ")
                .is_some_and(|r| r.starts_with(|c: char| c.is_ascii_digit()))
        })
        .count();
    let issue = match arg(args, "issue") {
        None | Some("" | "new") => "file the epic issue".to_owned(),
        Some(i) => format!("rewrite issue #{i} as the epic"),
    };
    let mut w = brief(facts);
    w.push(format!("- {issue}"));
    w.push(format!(
        "- file {n} slice issues in order and link them from the epic"
    ));
    w.push("- clean up — no code, no worktree".to_owned());
    Approval::new("Epic", w, Vec::new())
}

fn gate2(run: &Run, args: &[(String, String)], facts: &Facts) -> Result<Approval, Error> {
    let fast = facts.lane == "fast";
    let iw = match arg(args, "issue") {
        None | Some("" | "new") => "file the issue".to_owned(),
        Some(i) => format!("rewrite issue #{i}"),
    };
    let slug = run.slug;
    let branch = run
        .field("branch")?
        .unwrap_or_else(|| format!("<type>/{slug}"));
    let mut w = brief(facts);
    w.push(format!("- {iw}"));
    w.push(format!(
        "- create worktree .ferrule/worktrees/{slug} on branch {branch}"
    ));
    w.push(format!(
        "- create {} stage cards and spawn the implementer at {}{}",
        facts.stages,
        facts.first.as_deref().unwrap_or("s1"),
        if fast {
            " (commits and pushes on green, no per-stage stop)"
        } else {
            ""
        }
    ));
    w.push(format!(
        "- next stop: {}",
        if fast {
            "gate 4"
        } else {
            "per the stage-stop rules"
        }
    ));
    Ok(Approval::new(
        "Gate 2",
        w,
        lane_options(&facts.lane, "<stages, e.g. s1,s3>"),
    ))
}

fn gate4(run: &Run, args: &[(String, String)]) -> Result<Approval, Error> {
    let pr = match arg(args, "pr") {
        Some(p) if !p.is_empty() => Some(p.to_owned()),
        _ => run.field("pr")?,
    };
    let k = follow_ups(&run.layout.run_dir(run.slug)).len();
    let mut w = vec![format!(
        "- push, replace the draft's title and body, mark #{} ready",
        pr.as_deref().unwrap_or("<pr>")
    )];
    if k > 0 {
        w.push(format!("- file the {k} follow-up issues"));
    }
    w.push("- wait for CI, clean up".to_owned());
    let hold = "stop (hold, e.g. for a manual run first)";
    let (opts, answer) = if k > 0 {
        (
            strings(&["1. follow-ups: <numbers | none> — file only these (default: all)"]),
            format!("approve [<n>[: <value>] …] · change <what> · re-plan: <why> · {hold}"),
        )
    } else {
        (
            Vec::new(),
            format!("approve · change <what> · re-plan: <why> · {hold}"),
        )
    };
    let mut a = Approval::new("Gate 4", w, opts);
    a.answer = answer;
    Ok(a)
}

/// Records what the last stop showed: `spec-rev=` and `arch-rev=` of the
/// files in its approval marker, and decisions.md's line count.
fn approved(run: &mut Run) -> Result<Vec<String>, Error> {
    let mut out = String::from("approved");
    for path in approval::shown(run.conn, run.slug)? {
        let event = match path.file_name().and_then(|n| n.to_str()) {
            Some("spec-diff.md") => "spec-rev",
            Some("architecture.summary.md") => "arch-rev",
            _ => continue,
        };
        if let Some(r) = rev_of(&path) {
            let e = format!("{event}={r}");
            run.log(&e, &[])?;
            let _ = write!(out, " {e}");
        }
    }
    if let Ok(d) = std::fs::read_to_string(run.art("decisions.md")) {
        let n = d.bytes().filter(|&b| b == b'\n').count();
        let e = format!("decisions-seen={n}");
        run.log(&e, &[])?;
        let _ = write!(out, " {e}");
    }
    Ok(vec![out])
}

/// The one-line notice of a stage that auto-continues.
fn auto(run: &Run, args: &[(String, String)]) -> Result<Vec<String>, Error> {
    let sid = arg(args, "stage").ok_or(Error::Usage)?;
    let me: u32 = sid
        .trim_start_matches('s')
        .parse()
        .map_err(|_| Error::Usage)?;
    let next = run
        .stages()?
        .into_iter()
        .map(|(n, _, _)| n)
        .find(|&n| n > me)
        .map_or_else(|| "gate 4".to_owned(), |n| format!("s{n}"));
    let events = board::events(run.conn, run.slug, &[Name::Gauntlet])?;
    let (cov, sha) = events
        .iter()
        .rev()
        .find_map(|e| match e.typed() {
            Typed::Gauntlet {
                result: Verdict::Pass,
                cov,
                sha,
                ..
            } => Some((cov, sha)),
            _ => None,
        })
        .unwrap_or_default();
    let detail = std::fs::read_to_string(run.art("review.verdict.md"))
        .ok()
        .and_then(|v| {
            v.lines().find_map(|l| {
                l.strip_prefix("Spec changes — ")
                    .and_then(|r| r.rsplit_once(" detail").map(|(d, _)| d.to_owned()))
            })
        })
        .unwrap_or_else(|| "none".to_owned());
    Ok(vec![format!(
        "Landed {sid} · {} · review clean · gauntlet pass {} · pushed {} · spec detail: {detail} · next: {next} (auto-continue — say stop to hold)",
        run.slug,
        cov.unwrap_or("-"),
        sha.unwrap_or("-")
    )])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ut_replan_unseen_reads_the_stop_by_key() {
        let mut s = crate::store::Store::memory().unwrap();
        board::create(&mut s.conn, "r", State::Open, &[], "t").unwrap();
        board::insert_body(&s.conn, "r", "t", "replan=pre-authorized");
        board::insert_body(&s.conn, "r", "t", "stop gate=gate4 kind=approval");
        board::insert_body(&s.conn, "r", "t", "note replan=pre-authorized");
        let layout = Layout::new(PathBuf::from("."));
        let run = Run {
            conn: &mut s.conn,
            layout: &layout,
            slug: "r",
            now: "t",
        };
        assert!(!run.replan_unseen().unwrap());
        board::insert_body(run.conn, "r", "t", "replan=pre-authorized");
        assert!(run.replan_unseen().unwrap());
    }

    #[test]
    fn ut_reorder_keeps_items_under_their_header() {
        let lines = [
            "Review: x",
            "Needs user:",
            "- a",
            "Plan: p",
            "Follow-up 1: one",
            "Issue: t",
            "Confirm:",
            "- c",
        ]
        .map(String::from)
        .to_vec();
        assert_eq!(
            reorder(lines, Some("41")),
            [
                "Plan: p",
                "Issue: reuse #41 — t",
                "Follow-ups:",
                "- 1: one",
                "Confirm:",
                "- c",
                "Review: x",
                "Needs user:",
                "- a"
            ]
        );
    }
}
