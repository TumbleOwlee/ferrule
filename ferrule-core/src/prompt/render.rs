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
use crate::config::Config;
use crate::project::Layout;
use crate::prompt::{approval, lines};
use crate::remote;
use crate::run::stops::{self, EarlyStops};
use crate::store::Board;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Gate1,
    Arch,
    Epic,
    Gate2,
    Stage,
    Replan,
    Gate4,
    Question,
    Blocked,
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
            "gate4" => Kind::Gate4,
            "question" => Kind::Question,
            "blocked" => Kind::Blocked,
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
    /// The viewer step refused (a file unreadable, a summary over its
    /// cap); it reported why and answers its own exit code.
    Show,
    Sql(rusqlite::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Usage => f.write_str("the arguments lack what this prompt needs"),
            Error::Card => f.write_str("the run has no card"),
            Error::Show => f.write_str("the viewer step refused"),
            Error::Sql(e) => write!(f, "state database: {e}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Sql(e) => Some(e),
            Error::Usage | Error::Card | Error::Show => None,
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
    /// The configured tracker files an epic as a milestone.
    milestones: bool,
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

    /// The minors the newest review left, when no stop was logged since.
    fn minors_fixed(&self) -> rusqlite::Result<Option<u64>> {
        let events = board::events(self.conn, self.slug, &[Name::Review, Name::Stop])?;
        let Some(at) = events
            .iter()
            .rposition(|e| matches!(e.typed(), Typed::Review(_)))
        else {
            return Ok(None);
        };
        if events[at + 1..]
            .iter()
            .any(|e| matches!(e.typed(), Typed::Stop { .. }))
        {
            return Ok(None);
        }
        Ok(events[at]
            .arg("minors")
            .and_then(|m| m.parse::<u64>().ok())
            .filter(|&n| n > 0))
    }

    fn stages(&self) -> rusqlite::Result<Vec<(u32, String, State)>> {
        board::stages(self.conn, self.slug)
    }

    /// Whether a stage card of the run has no `commit=` logged yet.
    fn stage_left(&self) -> rusqlite::Result<bool> {
        for (_, id, _) in self.stages()? {
            if board::last_value(self.conn, &id, Name::Commit)?.is_none() {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

/// The run's state a stop's text depends on.
struct Facts {
    /// The early stops the run's brief or architecture size calls for.
    stops: EarlyStops,
    /// The run has a brief.
    has_brief: bool,
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

/// The run's `follow-up-<k>.md` files, by `k`.
fn follow_ups(dir: &Path) -> Vec<PathBuf> {
    artifacts::follow_ups(dir)
        .into_iter()
        .map(|(_, p)| p)
        .collect()
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
        let rows = artifacts::rows(&summary);
        Ok(Facts {
            stops: stops::early_stops(self.layout, self.slug),
            has_brief: spec.is_file(),
            brief_pending,
            want_spec: spec.is_file()
                && (brief_pending || rev_of(&spec).is_some_and(|r| r > since)),
            seen: number(self.value(Name::DecisionsSeen)?),
            stages: rows.len(),
            first: rows.first().map(artifacts::Row::sid),
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
            Kind::Stage => add(&mut show, "review.verdict.md"),
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
            Kind::Blocked | Kind::Auto | Kind::Approved => {}
        }
        Ok((show, quote))
    }
}

/// The note under the header when the viewer did not open the files.
/// The files shown are recorded as the stop's approval marker on the
/// render's own connection.
fn viewer_note(
    run: &mut Run,
    view: &mut dyn FnMut(&[PathBuf]) -> Option<Viewed>,
    show: &[PathBuf],
) -> Result<Option<String>, Error> {
    if show.is_empty() {
        return Ok(None);
    }
    let viewed = view(show);
    if viewed.is_some() {
        approval::record_in(run.conn, run.layout, show, run.now)?;
    }
    match viewed {
        Some(Viewed::Opened) => Ok(None),
        Some(Viewed::Failed) => Ok(Some("Viewer: failed — open the files manually".to_owned())),
        Some(Viewed::Unavailable(paths)) => {
            let paths: Vec<String> = paths.iter().map(|p| p.display().to_string()).collect();
            Ok(Some(format!(
                "Viewer: none available — open manually: {}",
                paths.join(" ")
            )))
        }
        None => Err(Error::Show),
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
    view: &mut dyn FnMut(&[PathBuf]) -> Option<Viewed>,
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
    view: &mut dyn FnMut(&[PathBuf]) -> Option<Viewed>,
) -> Result<Vec<String>, Error> {
    if board::state(conn, slug)?.is_none() {
        return Err(Error::Card);
    }
    let milestones = Config::load(&layout.root).is_ok_and(|c| {
        remote::issue_tracker(&c, &layout.root)
            .milestones()
            .is_some()
    });
    let mut run = Run {
        conn,
        layout,
        slug,
        now,
        milestones,
    };
    match kind {
        Kind::Approved => return approved(&mut run),
        Kind::Auto => return auto(&run, args),
        Kind::Question => return question(&mut run, args),
        Kind::Blocked => return blocked(&mut run, args),
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
    let note = viewer_note(&mut run, view, &show)?;
    let approval = approval_text(&run, kind, args, &facts)?;
    let minors = if matches!(kind, Kind::Stage | Kind::Gate4) {
        run.minors_fixed()?
    } else {
        None
    };
    if kind == Kind::Stage {
        run.log("stop", &["kind=stage"])?;
    } else {
        let gate = format!("gate={}", kind.gate());
        run.log("stop", &["kind=approval", &gate])?;
    }
    let mut out = vec![header(&approval.title, slug, &show, kind, args)];
    out.extend(note);
    out.extend(quoted(&show, &quote, facts.seen, kind, args));
    if let Some(n) = minors {
        out.push(format!(
            "Minors fixed after the review, not re-reviewed: {n}"
        ));
    }
    out.push("Approving will:".to_owned());
    out.extend(approval.will);
    if !approval.options.is_empty() {
        out.push("Options:".to_owned());
        out.extend(approval.options);
    }
    out.push(format!("Answer: {}", approval.answer));
    Ok(out)
}

/// `<title> · <slug>[ · <files with rev>][ · stop: …]`.
fn header(
    title: &str,
    slug: &str,
    show: &[PathBuf],
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
    let n = artifacts::questions(&q);
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

fn sha(run: &mut Run, mismatch: &str) -> Result<Vec<String>, Error> {
    run.log("stop", &["kind=blocked"])?;
    let mut out = vec![
        format!("Question · {} · 1 decision", run.slug),
        format!("## 1. the push is refused: mismatch={mismatch}"),
        "The push stays refused until a stage commit lands on top of the current head.".to_owned(),
        "Options:".to_owned(),
    ];
    let left = run.stage_left()?;
    if left {
        out.push("- a) continue with the next stage, push after its commit".to_owned());
    }
    out.push("- b) fixed outside ferrule, push again".to_owned());
    out.push("- c) stop".to_owned());
    out.push(
        if left {
            "Answer: <1: a | b | c>"
        } else {
            "Answer: <1: b | c>"
        }
        .to_owned(),
    );
    Ok(out)
}

fn blocked(run: &mut Run, args: &[(String, String)]) -> Result<Vec<String>, Error> {
    match (
        arg(args, "local"),
        arg(args, "remote"),
        arg(args, "mismatch"),
    ) {
        (Some(local), Some(remote), None) => {
            run.log("stop", &["kind=blocked"])?;
            return Ok(vec![
                format!("Question · {} · 1 decision", run.slug),
                format!("## 1. the branch has diverged: local={local} remote={remote}"),
                "Rebase it through ferrule or resolve it outside ferrule; never force-push by hand."
                    .to_owned(),
                "Options:".to_owned(),
                "- a) rebase through ferrule".to_owned(),
                "- b) resolved outside ferrule, push again".to_owned(),
                "- c) stop".to_owned(),
                "Answer: <1: a | b | c>".to_owned(),
            ]);
        }
        (None, None, Some(mismatch)) => return sha(run, mismatch),
        (None, None, None) => {}
        _ => return Err(Error::Usage),
    }
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

/// Gate 2 and re-plan offer extra stops.
fn stop_options(example: &str) -> Vec<String> {
    vec![format!("1. also stop after: {example}")]
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
        Kind::Stage => stage(run, args)?,
        Kind::Replan => {
            let from = arg(args, "from").unwrap_or("");
            Approval::new(
                format!("Re-plan from {from}"),
                vec![
                    format!("- replace the cards of {from} onward"),
                    "- update the draft PR".to_owned(),
                    format!("- spawn a fresh implementer at {from}"),
                ],
                stop_options("<stages>"),
            )
        }
        Kind::Gate4 => gate4(run, args)?,
        _ => return Err(Error::Usage),
    })
}

/// Only the last stage has no next stage to spawn. Without `--stage`
/// there is no stage to tell apart, so the text stays the general one.
fn stage(run: &Run, args: &[(String, String)]) -> Result<Approval, Error> {
    let sid = arg(args, "stage").unwrap_or("");
    let me = sid.trim_start_matches('s').parse().ok();
    let last = match me {
        Some(me) => next_stage(run, me)?.is_none(),
        None => false,
    };
    let batch = match me {
        Some(me) => {
            let main = Config::load(&run.layout.root)
                .map_or_else(|_| "main".to_owned(), |c| c.main_branch);
            let wt = board::worktree(run.conn, run.layout, run.slug, run.slug)?;
            match crate::run::tier::review_point(run.conn, &wt, run.slug, me, &main)? {
                crate::run::tier::ReviewPoint::Batch(v) => Some(v),
                _ => None,
            }
        }
        None => None,
    };
    let commit = match batch {
        Some(v) => {
            let ids: Vec<String> = v.iter().map(|n| format!("s{n}")).collect();
            format!(
                "- commit the review fixes of {}, push, update the draft PR",
                ids.join(", ")
            )
        }
        None => format!("- commit {sid}, push, update the draft PR"),
    };
    Ok(Approval::new(
        format!("Stage {sid}"),
        vec![
            commit,
            if last {
                "- next stop: gate 4".to_owned()
            } else {
                "- spawn the next stage".to_owned()
            },
        ],
        Vec::new(),
    ))
}

fn gate1(args: &[(String, String)]) -> Approval {
    if arg(args, "size") == Some("epic") {
        return Approval::new(
            "Gate 1",
            strings(&[
                "- record the run as an epic (option 1) or a single run",
                "- hand the brief to the architect",
            ]),
            strings(&[
                "1. epic — a shared skeleton, one issue per slice (recommended)",
                "2. single run",
            ]),
        );
    }
    Approval::new(
        "Gate 1",
        strings(&["- hand the brief to the architect"]),
        Vec::new(),
    )
}

/// The architecture stop: the combined early stop also approves the brief
/// and lets gate 2 pass as a notice, and offers a stop at the plan too.
fn arch(facts: &Facts) -> Approval {
    let mut will = brief(facts);
    will.push(
        if facts.has_brief {
            "- hand brief and architecture to the planner"
        } else {
            "- hand the architecture to the planner"
        }
        .to_owned(),
    );
    let mut opts = Vec::new();
    if facts.stops == EarlyStops::Combined {
        will.push("- run gate 2 without a stop once the plan review is clean".to_owned());
        opts.push("1. stop at the plan too".to_owned());
    }
    Approval::new("Architecture", will, opts)
}

/// The epic stop's two filing lines: the epic itself, then its `n` slices.
fn epic_steps(issue: Option<&str>, n: usize, milestones: bool) -> [String; 2] {
    let existing = issue.filter(|i| !i.is_empty() && *i != "new");
    if milestones {
        let first = match existing {
            None => "file the milestone".to_owned(),
            Some(i) => format!("close issue #{i} and file the milestone"),
        };
        return [
            first,
            format!("file {n} slice issues in order in the milestone"),
        ];
    }
    let first = match existing {
        None => "file the epic issue".to_owned(),
        Some(i) => format!("rewrite issue #{i} as the epic"),
    };
    [
        first,
        format!("file {n} slice issues in order and link them from the epic"),
    ]
}

fn epic(run: &Run, args: &[(String, String)], facts: &Facts) -> Approval {
    let slices = std::fs::read_to_string(run.art("slices.md")).unwrap_or_default();
    let n = artifacts::slices(&slices).len();
    let [issue, slices] = epic_steps(arg(args, "issue"), n, run.milestones);
    let mut w = brief(facts);
    w.push(format!("- {issue}"));
    w.push(format!("- {slices}"));
    w.push("- clean up — no code, no worktree".to_owned());
    Approval::new("Epic", w, Vec::new())
}

fn gate2(run: &Run, args: &[(String, String)], facts: &Facts) -> Result<Approval, Error> {
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
        "- create {} stage cards and spawn the implementer at {}",
        facts.stages,
        facts.first.as_deref().unwrap_or("s1"),
    ));
    w.push("- next stop: per the stage-stop rules".to_owned());
    Ok(Approval::new(
        "Gate 2",
        w,
        stop_options("<stages, e.g. s1,s3>"),
    ))
}

/// `  - <k>: <title> → milestone "<m>"` for follow-up draft `text`, or
/// `→ no milestone`: only a slice run files in the milestone its draft names.
fn follow_up_line(k: u32, text: &str, slice_run: bool) -> String {
    let d = artifacts::draft(text);
    match d.milestone.filter(|_| slice_run) {
        Some(m) => format!("  - {k}: {} → milestone \"{m}\"", d.title),
        None => format!("  - {k}: {} → no milestone", d.title),
    }
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
        if run.milestones {
            let slice_run = run.field("kind")?.as_deref() == Some("slice");
            for (n, path) in artifacts::follow_ups(&run.layout.run_dir(run.slug)) {
                let text = std::fs::read_to_string(path).unwrap_or_default();
                w.push(follow_up_line(n, &text, slice_run));
            }
        }
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
    for e in seen(run.conn, run.layout, run.slug)? {
        run.log(&e, &[])?;
        let _ = write!(out, " {e}");
    }
    Ok(vec![out])
}

/// The events an approval records: `spec-rev=` and `arch-rev=` of the
/// files its marker shows, and decisions.md's line count. Read only, so a
/// gate can log them in the transaction that records it.
pub(crate) fn seen(
    conn: &Connection,
    layout: &Layout,
    slug: &str,
) -> rusqlite::Result<Vec<String>> {
    let mut out = Vec::new();
    for path in approval::shown(conn, slug)? {
        let event = match path.file_name().and_then(|n| n.to_str()) {
            Some("spec-diff.md") => "spec-rev",
            Some("architecture.summary.md") => "arch-rev",
            _ => continue,
        };
        if let Some(r) = rev_of(&path) {
            out.push(format!("{event}={r}"));
        }
    }
    if let Ok(d) = std::fs::read_to_string(layout.run_dir(slug).join("decisions.md")) {
        out.push(format!("decisions-seen={}", artifacts::line_count(&d)));
    }
    Ok(out)
}

/// The stage after `me`, if any.
fn next_stage(run: &Run, me: u32) -> Result<Option<u32>, Error> {
    Ok(run
        .stages()?
        .into_iter()
        .map(|(n, _, _)| n)
        .find(|&n| n > me))
}

/// The one-line notice of a stage that auto-continues: its coverage comes from
/// the newest `gauntlet=pass` and its commit from the newest `pushed=`, each
/// after the run's last `rebase=`.
fn auto(run: &Run, args: &[(String, String)]) -> Result<Vec<String>, Error> {
    let sid = arg(args, "stage").ok_or(Error::Usage)?;
    let me: u32 = sid
        .trim_start_matches('s')
        .parse()
        .map_err(|_| Error::Usage)?;
    let next = next_stage(run, me)?.map_or_else(|| "gate 4".to_owned(), |n| format!("s{n}"));
    let events = board::events(
        run.conn,
        run.slug,
        &[Name::Gauntlet, Name::Pushed, Name::Rebase],
    )?;
    let after: Vec<_> = events
        .iter()
        .rev()
        .take_while(|e| !matches!(e.typed(), Typed::Rebase { .. }))
        .collect();
    let sha = after.iter().find_map(|e| match e.typed() {
        Typed::Pushed { sha } => Some(sha),
        _ => None,
    });
    let cov = after.iter().find_map(|e| match e.typed() {
        Typed::Gauntlet {
            result: Verdict::Pass,
            cov,
            ..
        } => Some(cov),
        _ => None,
    });
    let detail = std::fs::read_to_string(run.art("review.verdict.md"))
        .ok()
        .and_then(|v| {
            v.lines().find_map(|l| {
                l.strip_prefix("Spec changes — ")
                    .and_then(|r| r.rsplit_once(" detail").map(|(d, _)| d.to_owned()))
            })
        })
        .unwrap_or_else(|| "none".to_owned());
    let minors = run
        .minors_fixed()?
        .map_or_else(String::new, |n| format!(" · minors fixed {n}"));
    Ok(vec![format!(
        "Landed {sid} · {} · review clean · gauntlet pass {} · pushed {} · spec detail: {detail}{minors} · next: {next} (auto-continue — say stop to hold)",
        run.slug,
        cov.flatten().unwrap_or("-"),
        sha.unwrap_or("-")
    )])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// GW-R-028 — a caller other than clap gets the usage error.
    #[test]
    fn ut_blocked_refuses_one_head_alone() {
        let mut s = crate::store::Store::memory().unwrap();
        board::create(&mut s.conn, "r", State::Open, &[], "t").unwrap();
        let layout = Layout::new(PathBuf::from("."));
        let mut run = Run {
            conn: &mut s.conn,
            layout: &layout,
            slug: "r",
            now: "t",
            milestones: false,
        };
        for key in ["local", "remote"] {
            let args = [(key.to_owned(), "abc1234".to_owned())];
            assert!(matches!(blocked(&mut run, &args), Err(Error::Usage)));
        }
    }

    /// GW-R-088 — a caller other than clap gets the usage error.
    #[test]
    fn ut_blocked_refuses_mismatch_with_a_head() {
        let mut s = crate::store::Store::memory().unwrap();
        board::create(&mut s.conn, "r", State::Open, &[], "t").unwrap();
        let layout = Layout::new(PathBuf::from("."));
        let mut run = Run {
            conn: &mut s.conn,
            layout: &layout,
            slug: "r",
            now: "t",
            milestones: false,
        };
        let pair = |k: &str, v: &str| (k.to_owned(), v.to_owned());
        let sets = [
            vec![pair("mismatch", "card"), pair("local", "abc1234")],
            vec![pair("mismatch", "card"), pair("remote", "abc1234")],
            vec![
                pair("mismatch", "card"),
                pair("local", "abc1234"),
                pair("remote", "abc1234"),
            ],
        ];
        for args in sets {
            assert!(matches!(blocked(&mut run, &args), Err(Error::Usage)));
        }
    }

    /// The epic stop names a milestone on a tracker with them, an issue and its list otherwise.
    #[test]
    fn ut_epic_steps_name_a_milestone_or_an_issue() {
        assert_eq!(
            epic_steps(None, 3, true),
            [
                "file the milestone",
                "file 3 slice issues in order in the milestone"
            ]
        );
        assert_eq!(
            epic_steps(Some("12"), 3, true),
            [
                "close issue #12 and file the milestone",
                "file 3 slice issues in order in the milestone"
            ]
        );
        assert_eq!(epic_steps(Some("new"), 2, true)[0], "file the milestone");
        assert_eq!(
            epic_steps(None, 3, false),
            [
                "file the epic issue",
                "file 3 slice issues in order and link them from the epic"
            ]
        );
        assert_eq!(
            epic_steps(Some("12"), 3, false)[0],
            "rewrite issue #12 as the epic"
        );
    }

    /// RU-R-029 — gate 4 shows each follow-up's milestone, or that it has none.
    #[test]
    fn ut_gate4_shows_each_follow_ups_milestone() {
        let text = "# Share the helper\n\nMilestone: Epic one\nBody\n";
        assert_eq!(
            follow_up_line(1, text, true),
            "  - 1: Share the helper → milestone \"Epic one\""
        );
        assert_eq!(
            follow_up_line(2, "# Other\n\nMilestone: none\nBody\n", true),
            "  - 2: Other → no milestone"
        );
        assert_eq!(
            follow_up_line(3, "# Plain\n\nBody\n", true),
            "  - 3: Plain → no milestone"
        );
        // A run that is no slice run files in no milestone, whatever the draft names.
        assert_eq!(
            follow_up_line(1, text, false),
            "  - 1: Share the helper → no milestone"
        );
    }

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
            milestones: false,
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
