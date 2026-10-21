//! Starting a run and recording its approvals: the board work the
//! orchestrator does around each gate, so no card is written by hand.
//!
//! Gate 2 creates the stage cards from `plan.md`'s `## Shared` stage table,
//! the one plan read the orchestrator makes:
//!
//! ```text
//! | Stage | Title | Files | Blocked by |
//! |---|---|---|---|
//! | s0 | Land spec | docs/specs/modbus/requirements.md | — |
//! | s1 | Add backoff | ferrowl-modbus/src/client_core.rs, ferrowl-modbus/src/lib.rs | s0 |
//! ```

use std::path::Path;

use rusqlite::Connection;

use crate::card::board::{self, NewCard, State};
use crate::card::typed::{Change, Grant, Name, Typed};
use crate::config::Config;
use crate::git;
use crate::issue::request::{self, Part};
use crate::pr::publish::{self, Target};
use crate::project::Layout;
use crate::prompt;
use crate::refusal::Refusal;
use crate::remote::{self, Item, ItemRef};
use crate::store::Board;

/// Why `run init`, `gate approve` or `stage replan` refused.
#[derive(Debug)]
pub enum GateRefusal {
    /// The run already has a card or stage cards.
    Exists,
    /// No such run or stage card.
    Card,
    /// plan.md is missing from the run directory.
    NoPlan,
    /// plan.md has no stage table (from the re-planned stage on).
    NoStages,
    /// `--issue` is neither `new` nor a number.
    Usage,
    /// `git worktree add` failed.
    Worktree(git::GitError),
    /// The parent card cannot move to inprogress.
    Move,
    /// No replan was started since the last approval.
    NoReplan,
    /// A gate approved before the one it follows: the missing gate and
    /// the flags its approval takes.
    Order {
        missing: &'static str,
        flags: &'static str,
    },
    /// A gate approved while the parent card is in a state it is not
    /// approved in.
    State { gate: Gate, state: State },
    /// `gate1 --epic` on a run past gate 1.
    Epic,
    /// gate2's `--branch` differs from the branch the card records.
    Branch(String),
    /// The new plan redefines a stage already committed.
    Committed(u32),
}

impl GateRefusal {
    /// The reason word.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            GateRefusal::Exists => "exists",
            GateRefusal::Card => "card",
            GateRefusal::NoPlan => "no-plan",
            GateRefusal::NoStages => "no-stages",
            GateRefusal::Usage => "usage",
            GateRefusal::Worktree(_) => "worktree",
            GateRefusal::Move => "move",
            GateRefusal::NoReplan => "no-replan",
            GateRefusal::Order { .. } => "order",
            GateRefusal::State { .. } => "state",
            GateRefusal::Epic => "epic",
            GateRefusal::Branch(_) => "branch",
            GateRefusal::Committed(_) => "committed",
        }
    }

    /// The `<verb>=refused` line of run `slug`.
    #[must_use]
    pub fn refusal(&self, verb: &'static str, slug: &str) -> Refusal {
        let r = Refusal::new(verb, self.as_str());
        match self {
            GateRefusal::Exists => r.hint(format!(
                "the run exists; resume it with `ferrule run reconcile {slug}`"
            )),
            GateRefusal::Card => Refusal::card(verb),
            GateRefusal::NoPlan => r.hint(format!("write .ferrule/runs/{slug}/plan.md first")),
            GateRefusal::NoStages => r.hint("add the `## Shared` stage table to plan.md"),
            GateRefusal::Usage => r.hint("--issue takes `new` or an issue number"),
            GateRefusal::Worktree(e) => r
                .detail(&e.stderr)
                .hint("git could not add the worktree; check `git worktree list` and --branch"),
            GateRefusal::Move => r.hint(format!(
                "the parent card cannot move to inprogress; see `ferrule board show {slug}`"
            )),
            GateRefusal::NoReplan => r.hint(format!(
                "no replan is pending; start one: ferrule stage replan {slug} start"
            )),
            GateRefusal::Order { missing, flags } => r.field("missing", missing).hint(format!(
                "approve {missing} first: ferrule gate approve {slug} {missing}{flags}"
            )),
            GateRefusal::State { gate, state } => {
                let states: Vec<&str> = gate.states().iter().map(|s| s.as_str()).collect();
                r.field("state", state).hint(format!(
                    "{} is approved only while the parent card is {}; see `ferrule run status {slug}`",
                    gate.as_str(),
                    states.join(" or ")
                ))
            }
            GateRefusal::Epic => r.hint(format!(
                "the run is past gate 1 (arch or gate2 recorded, or stage cards exist); \
                 an epic is chosen at gate 1 only; see `ferrule run status {slug}`"
            )),
            GateRefusal::Branch(card) => r.field("branch", card).hint(format!(
                "the run's branch is {card}; drop --branch or pass --branch {card}"
            )),
            GateRefusal::Committed(n) => r.field("stage", format!("s{n}")).hint(format!(
                "s{n} is committed and stays; number the new plan's stages around it"
            )),
        }
    }
}

#[derive(Debug)]
pub enum Error {
    /// The status-line refusal.
    Refused(GateRefusal),
    Remote(remote::Error),
    Publish(publish::Error),
    Board(board::BatchError),
    Io(std::io::Error),
    Sql(rusqlite::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Refused(r) => write!(f, "refused: {}", r.as_str()),
            Error::Remote(e) => write!(f, "{e}"),
            Error::Publish(e) => write!(f, "{e}"),
            Error::Board(e) => write!(f, "{e}"),
            Error::Io(e) => write!(f, "{e}"),
            Error::Sql(e) => write!(f, "state database: {e}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Refused(_) => None,
            Error::Remote(e) => Some(e),
            Error::Publish(e) => Some(e),
            Error::Board(e) => Some(e),
            Error::Io(e) => Some(e),
            Error::Sql(e) => Some(e),
        }
    }
}

impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Self {
        Error::Sql(e)
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

impl From<remote::Error> for Error {
    fn from(e: remote::Error) -> Self {
        Error::Remote(e)
    }
}

impl From<publish::Error> for Error {
    fn from(e: publish::Error) -> Self {
        Error::Publish(e)
    }
}

fn write_lines(path: &Path, lines: &[String]) -> std::io::Result<()> {
    if lines.is_empty() {
        return Ok(());
    }
    std::fs::write(path, format!("{}\n", lines.join("\n")))
}

/// The comments of an issue, as `issue request --comments` prints them.
fn comments(item: &Item) -> String {
    let text = item.text();
    let after = text.split_once(" comment(s) ---\n").map_or("", |(_, r)| r);
    after.strip_prefix('\n').unwrap_or(after).to_owned()
}

/// Creates the parent card and the run's directory. With an issue the user
/// named, stores its request and any brief it carries; a slice issue also
/// gets its `**Part of epic**` line and the epic as it stands, and the card
/// records `kind: slice` and `epic`. Returns the epic of a slice.
pub fn init(
    db: &mut Board,
    layout: &Layout,
    config: &Config,
    slug: &str,
    issue: Option<&ItemRef>,
    now: &str,
) -> Result<Option<String>, Error> {
    let conn = db.conn_mut();
    if board::state(conn, slug)?.is_some() || !board::stages(conn, slug)?.is_empty() {
        return Err(Error::Refused(GateRefusal::Exists));
    }
    let root = &layout.root;
    let tracker = remote::issue_tracker(config, root);
    let item = issue.map(|n| tracker.view(n)).transpose()?;
    let epic = item
        .as_ref()
        .and_then(|i| request::part(&i.body, Part::Epic).into_iter().next());
    let epic_item = epic
        .as_deref()
        .map(|e| ItemRef::parse(e).and_then(|e| tracker.view(&e)))
        .transpose()?;
    let mut fields = Vec::new();
    if let Some(e) = &epic {
        fields.push(("kind".to_owned(), "slice".to_owned()));
        fields.push(("epic".to_owned(), e.clone()));
    }
    board::create_all(
        conn,
        &[NewCard {
            id: slug.to_owned(),
            state: State::Open,
            fields,
            log: Vec::new(),
        }],
        now,
    )
    .map_err(Error::Board)?;
    let dir = layout.run_dir(slug);
    std::fs::create_dir_all(&dir)?;
    if let Some(item) = &item {
        write_lines(
            &dir.join("request.md"),
            &request::part(&item.body, Part::Request),
        )?;
        write_lines(
            &dir.join("issue-spec.md"),
            &request::part(&item.body, Part::Spec),
        )?;
        if let Some(e) = &epic_item {
            write_lines(
                &dir.join("slice-line.md"),
                &request::part(&item.body, Part::SliceLine),
            )?;
            let mut text = request::part(&e.body, Part::Spec).join("\n");
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(&comments(e));
            if !text.is_empty() {
                std::fs::write(dir.join("epic.md"), text)?;
            }
        }
    }
    Ok(epic)
}

/// A gate the user approved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gate {
    Gate1,
    Arch,
    Epic,
    Gate2,
    /// A stage stop: only what was shown is recorded.
    Stage,
    /// A re-plan stop: only what was shown is recorded.
    Replan,
    /// A meaningful spec change: only what was shown is recorded.
    Delta,
    Gate3,
    Gate4,
}

impl Gate {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Gate::Gate1 => "gate1",
            Gate::Arch => "arch",
            Gate::Epic => "epic",
            Gate::Gate2 => "gate2",
            Gate::Stage => "stage",
            Gate::Replan => "replan",
            Gate::Delta => "delta",
            Gate::Gate3 => "gate3",
            Gate::Gate4 => "gate4",
        }
    }

    /// The parent card field the approval dates; `None` for the stops
    /// that recur.
    #[must_use]
    pub fn field(self) -> Option<&'static str> {
        match self {
            Gate::Gate1 => Some("gate1"),
            Gate::Arch | Gate::Epic => Some("arch"),
            Gate::Gate2 => Some("gate2"),
            Gate::Gate3 => Some("gate3"),
            Gate::Gate4 => Some("gate4"),
            Gate::Stage | Gate::Replan | Gate::Delta => None,
        }
    }

    /// The parent card states the gate is approved in.
    #[must_use]
    pub fn states(self) -> &'static [State] {
        match self {
            Gate::Gate1 | Gate::Arch | Gate::Epic => &[State::Open],
            // Re-approving gate2 resumes a run that died after it.
            Gate::Gate2 => &[State::Open, State::InProgress],
            Gate::Stage => &[State::InProgress],
            Gate::Replan | Gate::Delta | Gate::Gate3 | Gate::Gate4 => {
                &[State::InProgress, State::InReview]
            }
        }
    }
}

/// Whether `v` holds something: a date, or `no-arch`/`skipped`.
fn has(v: Option<&str>) -> bool {
    v.is_some_and(|v| !v.trim().is_empty())
}

/// The gate `gate` follows when it is not approved yet, and the flags its
/// approval takes: `epic` needs `kind: epic` (gate1 --epic), gate2 an
/// `arch` (a date, `no-arch` or `skipped`), every later gate `gate2`.
/// gate1 and arch have none: a non-behavior run starts at arch.
fn missing(
    conn: &Connection,
    slug: &str,
    gate: Gate,
) -> rusqlite::Result<Option<(&'static str, &'static str)>> {
    let field = |k: &str| board::field(conn, slug, k);
    Ok(match gate {
        Gate::Gate1 | Gate::Arch => None,
        Gate::Epic => (field("kind")?.as_deref() != Some("epic")).then_some(("gate1", " --epic")),
        Gate::Gate2 => (!has(field("arch")?.as_deref())).then_some(("arch", "")),
        Gate::Stage | Gate::Replan | Gate::Delta | Gate::Gate3 | Gate::Gate4 => {
            (!has(field("gate2")?.as_deref())).then_some(("gate2", ""))
        }
    })
}

/// One row of the `## Shared` stage table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanStage {
    pub n: u32,
    pub title: String,
    pub files: Vec<String>,
    pub blocked_by: Vec<String>,
}

fn cells_list(cell: &str) -> Vec<String> {
    cell.split(',')
        .map(str::trim)
        .filter(|c| !c.is_empty() && *c != "—" && *c != "-")
        .map(|c| c.trim_matches('`').to_owned())
        .collect()
}

/// The stage table of `plan.md`'s `## Shared` section.
#[must_use]
pub fn plan_stages(plan: &str) -> Vec<PlanStage> {
    let mut fence = false;
    let mut on = false;
    let mut out = Vec::new();
    for l in plan.lines() {
        if l.starts_with("```") {
            fence = !fence;
        }
        if fence {
            continue;
        }
        if l.starts_with("## ") {
            on = l == "## Shared";
            continue;
        }
        if !on {
            continue;
        }
        let cells: Vec<&str> = l.split('|').map(str::trim).collect();
        if cells.len() < 6 || !cells[0].is_empty() {
            continue;
        }
        let Some(n) = cells[1].strip_prefix('s').and_then(|n| n.parse().ok()) else {
            continue;
        };
        out.push(PlanStage {
            n,
            title: cells[2].to_owned(),
            files: cells_list(cells[3]),
            blocked_by: cells_list(cells[4]),
        });
    }
    out
}

/// The stages plan.summary.md's `Stop` column marks `yes`.
#[must_use]
pub fn summary_stops(summary: &str) -> Vec<u32> {
    crate::artifacts::rows(summary)
        .iter()
        .filter(|r| r.stop().is_some())
        .map(|r| r.n)
        .collect()
}

/// The new stage cards of `stages`, open, with the settled stops.
#[must_use]
pub fn stage_cards(slug: &str, stages: &[PlanStage], stops: &[u32], branch: &str) -> Vec<NewCard> {
    stages
        .iter()
        .map(|s| {
            let deps: Vec<String> = s.blocked_by.iter().map(|d| format!("{slug}.{d}")).collect();
            NewCard {
                id: format!("{slug}.s{}", s.n),
                state: State::Open,
                fields: vec![
                    ("title".to_owned(), s.title.clone()),
                    ("blocked-by".to_owned(), format!("[{}]", deps.join(", "))),
                    ("files".to_owned(), format!("[{}]", s.files.join(", "))),
                    ("branch".to_owned(), branch.to_owned()),
                    (
                        "stop".to_owned(),
                        if stops.contains(&s.n) { "yes" } else { "no" }.to_owned(),
                    ),
                ],
                log: Vec::new(),
            }
        })
        .collect()
}

/// `stops=s1,s3`.
#[must_use]
pub fn stop_arg(v: Option<&str>) -> Vec<u32> {
    v.unwrap_or("")
        .split(',')
        .filter_map(|s| s.trim().strip_prefix('s')?.parse().ok())
        .collect()
}

/// What an approval did, for the status line.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Approved {
    /// The `prompt render approved` record (`spec-rev=… decisions-seen=…`).
    pub record: String,
    pub issue: Option<u64>,
    pub worktree: Option<String>,
    pub stages: Option<usize>,
    /// The gate was approved already: its date is kept.
    pub recorded: bool,
}

/// The last `key=` of `args`.
fn arg<'a>(args: &'a [(String, String)], key: &str) -> Option<&'a str> {
    args.iter()
        .rev()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

/// Gate 2's work, in order: the issue filed or rewritten and the worktree
/// and branch (both idempotent), then one board transaction: the dates,
/// branch and lane, the parent moved, the stage cards created. A refused
/// move leaves no gate 2 field and no stage card.
#[allow(clippy::too_many_arguments)]
fn approve_gate2(
    conn: &mut Connection,
    layout: &Layout,
    config: &Config,
    slug: &str,
    args: &[(String, String)],
    now: &str,
    gate1_pending: bool,
    out: &mut Approved,
) -> Result<(), Error> {
    let today = now.get(..10).unwrap_or(now);
    let plan = std::fs::read_to_string(layout.run_dir(slug).join("plan.md"))
        .map_err(|_| Error::Refused(GateRefusal::NoPlan))?;
    let stages = plan_stages(&plan);
    if stages.is_empty() {
        return Err(Error::Refused(GateRefusal::NoStages));
    }
    let reuse = match arg(args, "issue") {
        None | Some("" | "new") => None,
        Some(n) => Some(n.parse().map_err(|_| Error::Refused(GateRefusal::Usage))?),
    };
    let recorded = board::field(conn, slug, "branch")?.filter(|b| !b.is_empty());
    if let (Some(card), Some(asked)) = (&recorded, arg(args, "branch"))
        && card != asked
    {
        return Err(Error::Refused(GateRefusal::Branch(card.clone())));
    }
    let branch = recorded
        .or_else(|| arg(args, "branch").map(str::to_owned))
        .unwrap_or_else(|| format!("feat/{slug}"));
    let root = layout.root.clone();
    let mut run = publish::Run {
        conn,
        layout,
        config,
        root: &root,
        slug,
        now,
        host: remote::code_host(config, &root),
        tracker: remote::issue_tracker(config, &root),
    };
    out.issue = Some(run.file(Target::Run, reuse)?.0);
    let wt = super::worktree(conn, layout, slug, slug)?;
    super::ensure_worktree(layout, &wt, &branch, &config.main_branch)
        .map_err(|e| Error::Refused(GateRefusal::Worktree(e)))?;
    out.worktree = Some(
        wt.strip_prefix(&layout.root)
            .unwrap_or(&wt)
            .display()
            .to_string(),
    );
    let mut fields = vec![("branch".to_owned(), branch.clone())];
    if gate1_pending {
        fields.push(("gate1".to_owned(), today.to_owned()));
    }
    if !out.recorded {
        fields.push(("gate2".to_owned(), today.to_owned()));
    }
    if let Some(lane) = arg(args, "lane") {
        fields.push(("lane".to_owned(), lane.to_owned()));
    }
    let summary =
        std::fs::read_to_string(layout.run_dir(slug).join("plan.summary.md")).unwrap_or_default();
    let mut stops = summary_stops(&summary);
    stops.extend(stop_arg(arg(args, "stops")));
    let existing = board::stages(conn, slug)?;
    let fresh: Vec<PlanStage> = stages
        .into_iter()
        .filter(|s| !existing.iter().any(|(n, _, _)| *n == s.n))
        .collect();
    let cards = stage_cards(slug, &fresh, &stops, &branch);
    match board::set_move_create(conn, slug, &fields, State::InProgress, &cards, now) {
        Ok(()) => {}
        Err(board::BatchError::Move(_, board::MoveError::Sql(e))) => return Err(Error::Sql(e)),
        Err(board::BatchError::Move(..)) => return Err(Error::Refused(GateRefusal::Move)),
        Err(e) => return Err(Error::Board(e)),
    }
    out.stages = Some(board::stages(conn, slug)?.len());
    Ok(())
}

/// Records the approval of `gate`: its date and lane, what was shown, and
/// at gate 2 the issue, the worktree, the parent's move and the stage
/// cards. Refused before anything is written when a gate it follows is
/// not approved or the parent card is in a state the gate is not approved
/// in. Approving a gate again keeps its date (`recorded`).
pub fn approve(
    db: &mut Board,
    layout: &Layout,
    config: &Config,
    gate: Gate,
    slug: &str,
    args: &[(String, String)],
    now: &str,
) -> Result<Approved, Error> {
    let conn = db.conn_mut();
    let Some(state) = board::state(conn, slug)? else {
        return Err(Error::Refused(GateRefusal::Card));
    };
    if let Some((missing, flags)) = missing(conn, slug, gate)? {
        return Err(Error::Refused(GateRefusal::Order { missing, flags }));
    }
    if !gate.states().contains(&state) {
        return Err(Error::Refused(GateRefusal::State { gate, state }));
    }
    let today = now.get(..10).unwrap_or(now);
    let gate1_pending = !board::is_date(board::field(conn, slug, "gate1")?.as_deref());
    let mut out = Approved {
        recorded: match gate.field() {
            Some(f) => board::is_date(board::field(conn, slug, f)?.as_deref()),
            None => false,
        },
        ..Approved::default()
    };
    let mut fields = Vec::new();
    if gate1_pending && matches!(gate, Gate::Arch | Gate::Epic) {
        fields.push(("gate1".to_owned(), today.to_owned()));
    }
    if let Some(f) = gate.field()
        && !out.recorded
        && gate != Gate::Gate2
    {
        fields.push((f.to_owned(), today.to_owned()));
    }
    if gate == Gate::Gate1
        && arg(args, "kind") == Some("epic")
        && board::field(conn, slug, "kind")?.as_deref() != Some("epic")
    {
        let past = has(board::field(conn, slug, "arch")?.as_deref())
            || has(board::field(conn, slug, "gate2")?.as_deref())
            || !board::stages(conn, slug)?.is_empty();
        if past {
            return Err(Error::Refused(GateRefusal::Epic));
        }
        fields.push(("kind".to_owned(), "epic".to_owned()));
        fields.push(("lane".to_owned(), String::new()));
    }
    let seen = prompt::render::seen(conn, layout, slug)?;
    if gate == Gate::Gate2 {
        // Its issue, worktree and cards come first, each idempotent, and
        // write in their own transaction; what was shown follows.
        approve_gate2(
            conn,
            layout,
            config,
            slug,
            args,
            now,
            gate1_pending,
            &mut out,
        )?;
        fields.clear();
    } else if let Some(lane) = arg(args, "lane") {
        fields.push(("lane".to_owned(), lane.to_owned()));
    }
    // The fields and what was shown in one transaction, the log lines
    // last: a refused line leaves the gate unapproved.
    db.write(|w| {
        if !fields.is_empty() {
            w.set_all(slug, &fields).map_err(|e| match e {
                board::CardError::Sql(e) => Error::Sql(e),
                e => Error::Board(board::BatchError::Card(slug.to_owned(), e)),
            })?;
        }
        for e in &seen {
            w.log(slug, e, &[], now).map_err(|e| match e {
                board::LogError::Sql(e) => Error::Sql(e),
                e => Error::Board(board::BatchError::Log(slug.to_owned(), e)),
            })?;
        }
        Ok::<_, Error>(())
    })?;
    out.record = seen.join(" ");
    Ok(out)
}

/// The stage a re-plan starts from: the lowest without a commit, or, when
/// every stage has one (asked at gate 4), the next free number.
fn replan_from(conn: &Connection, slug: &str) -> rusqlite::Result<u32> {
    let stages = board::stages(conn, slug)?;
    for (n, id, _) in &stages {
        if board::last_value(conn, id, Name::Commit)?.is_none() {
            return Ok(*n);
        }
    }
    Ok(stages.iter().map(|(n, _, _)| n + 1).max().unwrap_or(1))
}

/// Starts a re-plan: logs `replan from=s<n>` and snapshots plan.md as
/// plan.prev.md. Asked at gate 4, the parent moves back to inprogress.
/// The board changes are one transaction, the log line last: a refused
/// move clears no gate.
pub fn replan_start(db: &mut Board, layout: &Layout, slug: &str, now: &str) -> Result<u32, Error> {
    let dir = layout.run_dir(slug);
    let snapshot = dir.join("plan.prev.md.tmp");
    if dir.join("plan.md").is_file() {
        std::fs::copy(dir.join("plan.md"), &snapshot)?;
    }
    let started = db.write(|w| {
        let Some(st) = board::state(w.conn(), slug)? else {
            return Err(Error::Refused(GateRefusal::Card));
        };
        let from = replan_from(w.conn(), slug)?;
        // The run rejoins at gate 3 and gate 4: both are approved anew.
        let gates = [
            ("gate3".to_owned(), String::new()),
            ("gate4".to_owned(), String::new()),
        ];
        w.set_all(slug, &gates).map_err(|e| match e {
            board::CardError::Sql(e) => Error::Sql(e),
            _ => Error::Refused(GateRefusal::Card),
        })?;
        if st != State::InProgress {
            w.move_card(slug, State::InProgress, now)
                .map_err(|e| match e {
                    board::MoveError::Sql(e) => Error::Sql(e),
                    _ => Error::Refused(GateRefusal::Move),
                })?;
        }
        w.log(slug, "replan", &[format!("from=s{from}")], now)
            .map_err(|e| match e {
                board::LogError::Sql(e) => Error::Sql(e),
                _ => Error::Refused(GateRefusal::Card),
            })?;
        Ok(from)
    });
    match started {
        Ok(from) => {
            if snapshot.is_file() {
                std::fs::rename(&snapshot, dir.join("plan.prev.md"))?;
            }
            Ok(from)
        }
        Err(e) => {
            let _ = std::fs::remove_file(&snapshot);
            Err(e)
        }
    }
}

/// The re-plan approved: the stage cards from `from` on are replaced by the
/// new plan's, their `spec-delta … added` lines re-logged on the parent so
/// the IDs stay retired, and `replan=approved` logged.
pub fn replan_approve(
    db: &mut Board,
    layout: &Layout,
    slug: &str,
    stops: &[u32],
    now: &str,
) -> Result<(u32, usize), Error> {
    let conn = db.conn_mut();
    // Only a start logged after the last approval is pending; approving
    // again would recreate stages that have since progressed.
    let from: u32 = board::events(conn, slug, &[Name::Replan, Name::ReplanApproved])?
        .iter()
        .rev()
        .map(board::Event::typed)
        .take_while(|k| *k != Typed::ReplanApproved(Grant::Approved))
        .find_map(|k| match k {
            Typed::Replan { from: Some(f) } => f.strip_prefix('s')?.parse().ok(),
            _ => None,
        })
        .ok_or(Error::Refused(GateRefusal::NoReplan))?;
    let dir = layout.run_dir(slug);
    let plan = std::fs::read_to_string(dir.join("plan.md"))
        .map_err(|_| Error::Refused(GateRefusal::NoPlan))?;
    let fresh: Vec<PlanStage> = plan_stages(&plan)
        .into_iter()
        .filter(|s| s.n >= from)
        .collect();
    if fresh.is_empty() {
        return Err(Error::Refused(GateRefusal::NoStages));
    }
    let mut delete = Vec::new();
    let mut relog = Vec::new();
    for (n, id, _) in board::stages(conn, slug)? {
        if n < from {
            continue;
        }
        // A stage committed after the first uncommitted one is work on the
        // branch: its card stays, and the new plan may not reuse its number.
        if board::last_value(conn, &id, Name::Commit)?.is_some() {
            if fresh.iter().any(|s| s.n == n) {
                return Err(Error::Refused(GateRefusal::Committed(n)));
            }
            continue;
        }
        for e in board::events(conn, &id, &[Name::SpecDelta])? {
            if let Typed::SpecDelta {
                id: eid,
                change: Some(Change::Added),
                origin,
                ..
            } = e.typed()
            {
                let origin = origin.unwrap_or("impl");
                relog.push((
                    "spec-delta".to_owned(),
                    vec![
                        eid.to_owned(),
                        "added".to_owned(),
                        format!("origin={origin}"),
                        "--".to_owned(),
                        "retired".to_owned(),
                    ],
                ));
            }
        }
        delete.push(id);
    }
    relog.push(("replan=approved".to_owned(), Vec::new()));
    let branch = board::field(conn, slug, "branch")?.unwrap_or_default();
    let summary = std::fs::read_to_string(dir.join("plan.summary.md")).unwrap_or_default();
    let mut all_stops = summary_stops(&summary);
    all_stops.extend_from_slice(stops);
    let cards = stage_cards(slug, &fresh, &all_stops, &branch);
    board::replace_cards(conn, slug, &delete, &relog, &cards, now).map_err(Error::Board)?;
    Ok((from, cards.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ut_plan_stage_table() {
        let plan = "# p\n\n## Shared\n| Stage | Title | Files | Blocked by |\n|---|---|---|---|\n| s0 | Land spec | docs/specs/a.md | — |\n| s1 | Add backoff | `a.rs`, b.rs | s0 |\n\n## Stage s0: Land spec\n| s9 | not | the | table |\n";
        let s = plan_stages(plan);
        assert_eq!(s.len(), 2);
        assert_eq!(s[1].files, ["a.rs", "b.rs"]);
        assert_eq!(s[1].blocked_by, ["s0"]);
        assert!(s[0].blocked_by.is_empty());
    }

    #[test]
    fn ut_summary_stops() {
        let summary = "| Stage | What | Pins | Files | Stop |\n|---|---|---|---|---|\n| s0 | a | b | c | — |\n| s1 | a | b | c | yes — new variant |\n";
        assert_eq!(summary_stops(summary), [1]);
        assert_eq!(stop_arg(Some("s1, s3")), [1, 3]);
    }
}
