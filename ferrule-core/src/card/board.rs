//! The task board in the state database: a parent card per run and a stage
//! card per plan stage, each in one state, with frontmatter fields and an
//! append-only event log.

use std::collections::HashSet;
use std::fmt::{self, Write as _};
use std::path::Path;

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use crate::card::vocab::{self, Context, Reject};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Open,
    InProgress,
    InReview,
    Done,
}

impl State {
    pub const ALL: [State; 4] = [State::Open, State::InProgress, State::InReview, State::Done];

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            State::Open => "open",
            State::InProgress => "inprogress",
            State::InReview => "inreview",
            State::Done => "done",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        State::ALL.into_iter().find(|st| st.as_str() == s)
    }
}

impl fmt::Display for State {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A card id split into its run and, for a stage card, the stage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CardId<'a> {
    pub id: &'a str,
    pub run: &'a str,
    /// `3` of `<slug>.s3`.
    pub stage: Option<&'a str>,
}

impl<'a> CardId<'a> {
    /// `<slug>.s<n…>` is a stage card, anything else a parent card.
    #[must_use]
    pub fn parse(id: &'a str) -> Self {
        if let Some((run, stage)) = id.rsplit_once(".s")
            && stage.starts_with(|c: char| c.is_ascii_digit())
        {
            return CardId {
                id,
                run,
                stage: Some(stage),
            };
        }
        CardId {
            id,
            run: id,
            stage: None,
        }
    }
}

/// Frontmatter keys a parent card may carry; `id` is the card's own.
pub const PARENT_FIELDS: &[&str] = &[
    "kind",
    "epic",
    "slices",
    "issue",
    "pr",
    "branch",
    "lane",
    "gate1",
    "with-arch",
    "with-plan",
    "skipped",
    "arch",
    "no-arch",
    "gate2",
    "split",
    "upstream",
    "artifacts",
    "worktree",
];
/// Frontmatter keys a stage card may carry; `id` and `parent` are derived.
pub const STAGE_FIELDS: &[&str] = &["title", "blocked-by", "files", "branch", "worktree", "stop"];

fn allowed(card: &CardId, key: &str) -> bool {
    if card.stage.is_some() {
        STAGE_FIELDS.contains(&key)
    } else {
        PARENT_FIELDS.contains(&key)
    }
}

/// `[a, b]` as a frontmatter list holds it.
#[must_use]
pub fn list(value: &str) -> Vec<&str> {
    value
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect()
}

/// The current UTC minute, `YYYY-MM-DDTHH:MM`; `FERRULE_NOW` overrides it.
#[must_use]
pub fn now() -> String {
    if let Ok(now) = std::env::var("FERRULE_NOW") {
        return now;
    }
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let days = i64::try_from(secs / 86_400).unwrap_or(0);
    let (y, m, d) = civil(days);
    let rem = secs % 86_400;
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}",
        rem / 3600,
        rem % 3600 / 60
    )
}

/// Days since 1970-01-01 as a proleptic Gregorian date (Howard Hinnant's
/// `civil_from_days`).
fn civil(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

pub fn state(conn: &Connection, id: &str) -> rusqlite::Result<Option<State>> {
    let s: Option<String> = conn
        .query_row("SELECT state FROM cards WHERE id = ?1", [id], |r| r.get(0))
        .optional()?;
    Ok(s.and_then(|s| State::parse(&s)))
}

/// One frontmatter field of a card.
pub fn field(conn: &Connection, id: &str, key: &str) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT value FROM card_fields WHERE card = ?1 AND key = ?2",
        [id, key],
        |r| r.get(0),
    )
    .optional()
}

/// A card's frontmatter fields in their order.
pub fn fields(conn: &Connection, id: &str) -> rusqlite::Result<Vec<(String, String)>> {
    let mut st = conn.prepare("SELECT key, value FROM card_fields WHERE card = ?1 ORDER BY pos")?;
    st.query_map([id], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect()
}

/// A card's log lines as `(at, body)`, oldest first.
pub fn events(conn: &Connection, id: &str) -> rusqlite::Result<Vec<(String, String)>> {
    let mut st = conn.prepare("SELECT at, body FROM events WHERE card = ?1 ORDER BY id")?;
    st.query_map([id], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect()
}

/// Every log line of run `run`'s cards as `(card, at, body)`, by time;
/// lines at the same minute keep the parent card first, then the stages in
/// order.
pub fn run_events(conn: &Connection, run: &str) -> rusqlite::Result<Vec<(String, String, String)>> {
    let mut st = conn.prepare(
        "SELECT e.card, e.at, e.body FROM events e JOIN cards c ON c.id = e.card
         WHERE c.run = ?1
         ORDER BY e.at, c.stage IS NOT NULL, length(c.stage), c.stage, e.id",
    )?;
    st.query_map([run], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect()
}

/// The ID of a `spec-delta <ID> changed` line whose leading key fields
/// (`origin=`, `quote=`, in any order) include an `origin=`: a requirement
/// reworded after the issue was filed.
fn reworded_id(body: &str) -> Option<&str> {
    let mut words = body.splitn(4, ' ');
    let (event, id, change) = (words.next()?, words.next()?, words.next()?);
    if event != "spec-delta" || change != "changed" {
        return None;
    }
    let mut rest = words.next().unwrap_or("");
    loop {
        rest = rest.trim_start_matches(' ');
        let (key, value) = if let Some(v) = rest.strip_prefix("origin=") {
            ("origin", v)
        } else if let Some(v) = rest.strip_prefix("quote=") {
            ("quote", v)
        } else {
            return None;
        };
        if key == "origin" {
            return Some(id);
        }
        rest = match value.strip_prefix('"') {
            Some(quoted) => quoted.split_once('"').map_or("", |(_, after)| after),
            None => value.split_once(' ').map_or("", |(_, after)| after),
        };
    }
}

/// IDs of run `run`'s requirements reworded after filing, from the
/// `spec-delta … changed origin=…` lines on any of its cards.
pub fn reworded_ids(conn: &Connection, run: &str) -> rusqlite::Result<HashSet<String>> {
    Ok(run_events(conn, run)?
        .iter()
        .filter_map(|(_, _, b)| reworded_id(b))
        .map(str::to_owned)
        .collect())
}

/// The value of the last `<key>=` token in a card's log; the event word
/// counts, so `commit` finds `commit=<sha>`.
pub fn last_token(conn: &Connection, id: &str, key: &str) -> rusqlite::Result<Option<String>> {
    let prefix = format!("{key}=");
    Ok(events(conn, id)?
        .iter()
        .flat_map(|(_, body)| body.split_whitespace())
        .filter_map(|t| t.strip_prefix(&prefix))
        .next_back()
        .map(str::to_owned))
}

/// The `sha=` of every `gauntlet=pass` line on a card, oldest first.
pub fn pass_shas(conn: &Connection, id: &str) -> rusqlite::Result<Vec<String>> {
    Ok(events(conn, id)?
        .iter()
        .filter(|(_, body)| body.split_whitespace().any(|t| t == "gauntlet=pass"))
        .filter_map(|(_, body)| {
            body.split_whitespace()
                .filter_map(|t| t.strip_prefix("sha="))
                .next_back()
                .map(str::to_owned)
        })
        .collect())
}

/// The card as markdown: frontmatter (with its state) and the log.
pub fn show(conn: &Connection, id: &str) -> rusqlite::Result<Option<String>> {
    let Some(st) = state(conn, id)? else {
        return Ok(None);
    };
    let card = CardId::parse(id);
    let mut out = format!("---\nid: {id}\n");
    if card.stage.is_some() {
        let _ = writeln!(out, "parent: {}", card.run);
    }
    let _ = writeln!(out, "state: {st}");
    for (k, v) in fields(conn, id)? {
        let _ = writeln!(out, "{k}: {v}");
    }
    out.push_str("---\n");
    for (at, body) in events(conn, id)? {
        let _ = writeln!(out, "{at} {body}");
    }
    Ok(Some(out))
}

/// Every card, parent before its stages, runs in name order.
pub fn all(conn: &Connection) -> rusqlite::Result<Vec<(String, State)>> {
    let mut st = conn.prepare(
        "SELECT id, state FROM cards
         ORDER BY run, stage IS NOT NULL, length(stage), stage",
    )?;
    let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    let mut out = Vec::new();
    for row in rows {
        let (id, s) = row?;
        if let Some(s) = State::parse(&s) {
            out.push((id, s));
        }
    }
    Ok(out)
}

/// A run's stage cards as `(n, id, state)`, in stage order.
pub fn stages(conn: &Connection, run: &str) -> rusqlite::Result<Vec<(u32, String, State)>> {
    let mut st =
        conn.prepare("SELECT id, stage, state FROM cards WHERE run = ?1 AND stage IS NOT NULL")?;
    let rows = st.query_map([run], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (id, stage, st) = row?;
        if let (Ok(n), Some(st)) = (stage.parse(), State::parse(&st)) {
            out.push((n, id, st));
        }
    }
    out.sort_by_key(|(n, _, _)| *n);
    Ok(out)
}

#[derive(Debug)]
pub enum CardError {
    /// No card with that id.
    Card,
    /// A frontmatter key the card's kind does not carry.
    Field(String),
    /// The card exists already.
    Exists,
    Sql(rusqlite::Error),
}

impl From<rusqlite::Error> for CardError {
    fn from(e: rusqlite::Error) -> Self {
        CardError::Sql(e)
    }
}

/// Creates a card in `state` with its frontmatter fields.
pub fn create(
    conn: &mut Connection,
    id: &str,
    state: State,
    fields: &[(String, String)],
    now: &str,
) -> Result<(), CardError> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    insert(&tx, id, state, fields, now)?;
    tx.commit()?;
    Ok(())
}

/// A card to create with its first log lines.
#[derive(Debug, Clone)]
pub struct NewCard {
    pub id: String,
    pub state: State,
    pub fields: Vec<(String, String)>,
    /// `(event, args)`, each checked against the vocabulary.
    pub log: Vec<(String, Vec<String>)>,
}

#[derive(Debug)]
pub enum BatchError {
    Card(String, CardError),
    Log(String, LogError),
}

/// Creates every card and logs its lines, all or nothing.
pub fn create_all(conn: &mut Connection, cards: &[NewCard], now: &str) -> Result<(), BatchError> {
    let sql = |e: rusqlite::Error| BatchError::Card(String::new(), CardError::Sql(e));
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(sql)?;
    for c in cards {
        insert(&tx, &c.id, c.state, &c.fields, now)
            .map_err(|e| BatchError::Card(c.id.clone(), e))?;
        for (event, args) in &c.log {
            append(&tx, &c.id, event, args, now, vocab::READ_BUDGET)
                .map_err(|e| BatchError::Log(c.id.clone(), e))?;
        }
    }
    tx.commit().map_err(sql)
}

/// Deletes `delete`, logs `parent_log` on `parent` and creates `create`,
/// all or nothing.
pub fn replace_cards(
    conn: &mut Connection,
    parent: &str,
    delete: &[String],
    parent_log: &[(String, Vec<String>)],
    create: &[NewCard],
    now: &str,
) -> Result<(), BatchError> {
    let sql = |e: rusqlite::Error| BatchError::Card(String::new(), CardError::Sql(e));
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(sql)?;
    for (event, args) in parent_log {
        append(&tx, parent, event, args, now, vocab::READ_BUDGET)
            .map_err(|e| BatchError::Log(parent.to_owned(), e))?;
    }
    for id in delete {
        tx.execute("DELETE FROM cards WHERE id = ?1", [id])
            .map_err(sql)?;
    }
    for c in create {
        insert(&tx, &c.id, c.state, &c.fields, now)
            .map_err(|e| BatchError::Card(c.id.clone(), e))?;
        for (event, args) in &c.log {
            append(&tx, &c.id, event, args, now, vocab::READ_BUDGET)
                .map_err(|e| BatchError::Log(c.id.clone(), e))?;
        }
    }
    tx.commit().map_err(sql)
}

fn insert(
    conn: &Connection,
    id: &str,
    state: State,
    fields: &[(String, String)],
    now: &str,
) -> Result<(), CardError> {
    let card = CardId::parse(id);
    if let Some((k, _)) = fields.iter().find(|(k, _)| !allowed(&card, k)) {
        return Err(CardError::Field(k.clone()));
    }
    if self::state(conn, id)?.is_some() {
        return Err(CardError::Exists);
    }
    conn.execute(
        "INSERT INTO cards (id, run, stage, state, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![id, card.run, card.stage, state.as_str(), now],
    )?;
    for (pos, (k, v)) in fields.iter().enumerate() {
        conn.execute(
            "INSERT INTO card_fields (card, pos, key, value) VALUES (?1, ?2, ?3, ?4)",
            params![id, pos, k, v],
        )?;
    }
    Ok(())
}

/// Sets one frontmatter field; an empty value removes it.
pub fn set(conn: &mut Connection, id: &str, key: &str, value: &str) -> Result<(), CardError> {
    let card = CardId::parse(id);
    if !allowed(&card, key) {
        return Err(CardError::Field(key.to_owned()));
    }
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    if state(&tx, id)?.is_none() {
        return Err(CardError::Card);
    }
    if value.is_empty() {
        tx.execute(
            "DELETE FROM card_fields WHERE card = ?1 AND key = ?2",
            [id, key],
        )?;
    } else {
        tx.execute(
            "INSERT INTO card_fields (card, pos, key, value)
             VALUES (?1, (SELECT coalesce(max(pos), -1) + 1 FROM card_fields WHERE card = ?1), ?2, ?3)
             ON CONFLICT (card, key) DO UPDATE SET value = excluded.value",
            [id, key, value],
        )?;
    }
    tx.commit()?;
    Ok(())
}

#[derive(Debug)]
pub enum LogError {
    Card,
    Reject(Reject),
    Sql(rusqlite::Error),
}

impl From<rusqlite::Error> for LogError {
    fn from(e: rusqlite::Error) -> Self {
        LogError::Sql(e)
    }
}

/// Validates one event against the vocabulary and the card, and appends it.
pub fn log(
    conn: &mut Connection,
    id: &str,
    event: &str,
    args: &[String],
    now: &str,
) -> Result<(), LogError> {
    log_with_budget(conn, id, event, args, now, vocab::READ_BUDGET)
}

/// [`log`] with the project's `read` budget.
pub fn log_with_budget(
    conn: &mut Connection,
    id: &str,
    event: &str,
    args: &[String],
    now: &str,
    read_budget: usize,
) -> Result<(), LogError> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    append(&tx, id, event, args, now, read_budget)?;
    tx.commit()?;
    Ok(())
}

fn append(
    conn: &Connection,
    id: &str,
    event: &str,
    args: &[String],
    now: &str,
    read_budget: usize,
) -> Result<(), LogError> {
    let Some(st) = state(conn, id)? else {
        return Err(LogError::Card);
    };
    let card = CardId::parse(id);
    let reads: usize = conn.query_row(
        "SELECT count(*) FROM events WHERE card = ?1 AND name = 'read'",
        [id],
        |r| r.get(0),
    )?;
    let ctx = Context {
        stage: card.stage.is_some(),
        in_progress: st == State::InProgress,
        reads,
        read_budget,
    };
    let line = vocab::check(event, args, ctx).map_err(LogError::Reject)?;
    if now.chars().count() + 1 + line.body.chars().count() > vocab::MAX_LINE {
        return Err(LogError::Reject(Reject {
            event: line.name.trim_end_matches('=').to_owned(),
            reason: vocab::Reason::Length,
        }));
    }
    conn.execute(
        "INSERT INTO events (card, at, name, body) VALUES (?1, ?2, ?3, ?4)",
        params![id, now, line.name, line.body],
    )?;
    Ok(())
}

/// Why a move was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    Transition,
    BlockedBy,
    Commit,
    Unverified,
    Gate2,
}

impl Refusal {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Refusal::Transition => "transition",
            Refusal::BlockedBy => "blocked-by",
            Refusal::Commit => "commit",
            Refusal::Unverified => "unverified",
            Refusal::Gate2 => "gate2",
        }
    }
}

impl Refusal {
    /// How to get past the refusal, for the `hint=` of the status line.
    #[must_use]
    pub fn hint(self) -> &'static str {
        match self {
            Refusal::Transition => "see the card table in `ferrule guide board` for allowed moves",
            Refusal::BlockedBy => "move the cards in its blocked-by to done first",
            Refusal::Commit => "commit the stage with `ferrule card commit <card> -m <msg>` first",
            Refusal::Unverified => {
                "run `ferrule run gauntlet <slug>` so a gauntlet=pass covers the commit sha"
            }
            Refusal::Gate2 => "approve gate 2 first: `ferrule run approve <slug> gate2`",
        }
    }
}

#[derive(Debug)]
pub enum MoveError {
    Card,
    Refused { from: State, reason: Refusal },
    Sql(rusqlite::Error),
}

impl From<rusqlite::Error> for MoveError {
    fn from(e: rusqlite::Error) -> Self {
        MoveError::Sql(e)
    }
}

/// Moves a card, refusing what the board's rules forbid, and logs
/// `moved=<state>` in the same transaction. Moving to the current state is
/// a no-op.
pub fn move_card(conn: &mut Connection, id: &str, to: State, now: &str) -> Result<(), MoveError> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let Some(from) = state(&tx, id)? else {
        return Err(MoveError::Card);
    };
    if from == to {
        return Ok(());
    }
    if let Some(reason) = refusal(&tx, &CardId::parse(id), from, to)? {
        return Err(MoveError::Refused { from, reason });
    }
    tx.execute(
        "UPDATE cards SET state = ?1 WHERE id = ?2",
        [to.as_str(), id],
    )?;
    match append(
        &tx,
        id,
        &format!("moved={to}"),
        &[],
        now,
        vocab::READ_BUDGET,
    ) {
        Ok(()) => {}
        Err(LogError::Sql(e)) => return Err(MoveError::Sql(e)),
        Err(LogError::Card | LogError::Reject(_)) => return Err(MoveError::Card),
    }
    tx.commit()?;
    Ok(())
}

fn refusal(
    conn: &Connection,
    card: &CardId,
    from: State,
    to: State,
) -> rusqlite::Result<Option<Refusal>> {
    use State::{Done, InProgress, InReview, Open};
    if card.stage.is_some() {
        return Ok(match (from, to) {
            (Open, InProgress) => {
                let deps = field(conn, card.id, "blocked-by")?.unwrap_or_default();
                let mut blocked = false;
                for dep in list(&deps) {
                    let dep = if dep.contains('.') {
                        dep.to_owned()
                    } else {
                        format!("{}.{dep}", card.run)
                    };
                    blocked |= state(conn, &dep)? != Some(Done);
                }
                blocked.then_some(Refusal::BlockedBy)
            }
            (InProgress, InReview) | (InReview | Done, InProgress) => None,
            (InReview, Done) => match last_token(conn, card.id, "commit")? {
                None => Some(Refusal::Commit),
                Some(sha) => {
                    let verified = pass_shas(conn, card.run)?
                        .iter()
                        .any(|p| p.starts_with(&sha) || sha.starts_with(p.as_str()));
                    (!verified).then_some(Refusal::Unverified)
                }
            },
            _ => Some(Refusal::Transition),
        });
    }
    Ok(match (from, to) {
        (Open, InProgress) => field(conn, card.id, "gate2")?
            .is_none_or(|v| v.trim().is_empty())
            .then_some(Refusal::Gate2),
        (Open, InReview) => (field(conn, card.id, "kind")?.as_deref() != Some("epic"))
            .then_some(Refusal::Transition),
        (InProgress, InReview) | (InReview, InProgress) => None,
        _ => Some(Refusal::Transition),
    })
}

/// A card file of a markdown board: frontmatter and log lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CardFile {
    pub fields: Vec<(String, String)>,
    /// `(at, body)` per log line.
    pub events: Vec<(String, String)>,
}

/// Parses a card file: `---`-fenced `key: value` lines, then log lines.
#[must_use]
pub fn parse_card(text: &str) -> CardFile {
    let mut fields = Vec::new();
    let mut events = Vec::new();
    let mut fences = 0;
    for line in text.lines() {
        if fences < 2 && line == "---" {
            fences += 1;
            continue;
        }
        if fences == 1 {
            if let Some((k, v)) = line.split_once(':') {
                fields.push((k.trim().to_owned(), v.trim().to_owned()));
            }
        } else if fences == 2 && !line.trim().is_empty() {
            let (at, body) = line.split_once(' ').unwrap_or((line, ""));
            events.push((at.to_owned(), body.to_owned()));
        }
    }
    CardFile { fields, events }
}

/// The event name of a log body: its first word, through `=` when valued.
fn event_name(body: &str) -> &str {
    let w = body.split(' ').next().unwrap_or("");
    w.find('=').map_or(w, |i| &w[..=i])
}

#[derive(Debug)]
pub enum ImportError {
    /// A card file's frontmatter disagrees with its name, or carries a key
    /// its kind does not.
    Card {
        path: String,
        problem: String,
    },
    Exists(String),
    Io(std::io::Error),
    Sql(rusqlite::Error),
}

impl fmt::Display for ImportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ImportError::Card { path, problem } => write!(f, "{path}: {problem}"),
            ImportError::Exists(id) => write!(f, "card {id} is already on the board"),
            ImportError::Io(e) => write!(f, "{e}"),
            ImportError::Sql(e) => write!(f, "state database: {e}"),
        }
    }
}

impl From<rusqlite::Error> for ImportError {
    fn from(e: rusqlite::Error) -> Self {
        ImportError::Sql(e)
    }
}

/// Loads every card of a markdown board (`<board>/<state>/<id>.md`) in one
/// transaction: all or nothing. Returns the number of cards.
pub fn import(conn: &mut Connection, board: &Path, now: &str) -> Result<usize, ImportError> {
    let mut files = Vec::new();
    for st in State::ALL {
        let dir = board.join(st.as_str());
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(ImportError::Io(e)),
        };
        for entry in entries {
            let path = entry.map_err(ImportError::Io)?.path();
            if path.extension().is_some_and(|e| e == "md") {
                files.push((st, path));
            }
        }
    }
    files.sort_by(|a, b| a.1.cmp(&b.1));
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    for (st, path) in &files {
        let shown = path.display().to_string();
        let bad = |problem: String| ImportError::Card {
            path: shown.clone(),
            problem,
        };
        let id = path
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or_else(|| bad("file name is not UTF-8".to_owned()))?;
        let text = std::fs::read_to_string(path).map_err(ImportError::Io)?;
        let card = parse_card(&text);
        let cid = CardId::parse(id);
        let mut fields = Vec::new();
        for (k, v) in card.fields {
            match k.as_str() {
                "id" if v != id => return Err(bad(format!("id: {v} differs from the file name"))),
                "parent" if cid.stage.is_none() || v != cid.run => {
                    return Err(bad(format!("parent: {v} differs from the card id")));
                }
                "id" | "parent" => {}
                _ => fields.push((k, v)),
            }
        }
        match insert(&tx, id, *st, &fields, now) {
            Ok(()) => {}
            Err(CardError::Exists) => return Err(ImportError::Exists(id.to_owned())),
            Err(CardError::Field(k)) => {
                return Err(bad(format!("unknown frontmatter key {k}")));
            }
            Err(CardError::Sql(e)) => return Err(ImportError::Sql(e)),
            Err(CardError::Card) => unreachable!("insert never looks a card up by id"),
        }
        for (at, body) in card.events {
            tx.execute(
                "INSERT INTO events (card, at, name, body) VALUES (?1, ?2, ?3, ?4)",
                params![id, at, event_name(&body), body],
            )?;
        }
    }
    tx.commit()?;
    Ok(files.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ut_reworded_needs_origin_among_leading_keys() {
        assert_eq!(
            reworded_id("spec-delta MB-R-294 changed origin=user popup"),
            Some("MB-R-294")
        );
        assert_eq!(
            reworded_id("spec-delta MB-R-301 changed quote=\"a b\" origin=user box"),
            Some("MB-R-301")
        );
        assert_eq!(
            reworded_id("spec-delta MB-R-301 changed popup origin=user"),
            None
        );
        assert_eq!(reworded_id("spec-delta MB-R-301 added origin=user"), None);
    }
    use crate::store::Store;

    #[test]
    fn ut_card_ids() {
        assert_eq!(CardId::parse("demo.s3").stage, Some("3"));
        assert_eq!(CardId::parse("demo.s3").run, "demo");
        assert_eq!(CardId::parse("demo").stage, None);
        assert_eq!(CardId::parse("my.slug").stage, None);
        assert_eq!(CardId::parse("a.stuff").stage, None);
    }

    #[test]
    fn ut_civil_dates() {
        assert_eq!(civil(0), (1970, 1, 1));
        assert_eq!(civil(20_743), (2026, 10, 17));
        assert_eq!(civil(11_016), (2000, 2, 29));
    }

    #[test]
    fn ut_list_values() {
        assert_eq!(list("[demo.s0, s2]"), ["demo.s0", "s2"]);
        assert!(list("[]").is_empty());
    }

    #[test]
    fn ut_parse_card_file() {
        let c = parse_card("---\nid: x\ntitle: a: b\n---\n2026-10-03T09:00 spawn agent=impl\n\n");
        assert_eq!(c.fields[1], ("title".into(), "a: b".into()));
        assert_eq!(
            c.events,
            [("2026-10-03T09:00".into(), "spawn agent=impl".into())]
        );
    }

    fn store() -> Store {
        Store::memory().unwrap()
    }

    #[test]
    fn ut_move_logs_and_rolls_back_together() {
        let mut s = store();
        let f = |k: &str, v: &str| (k.to_owned(), v.to_owned());
        create(
            &mut s.conn,
            "r",
            State::Open,
            &[f("gate2", "2026-10-03")],
            "t",
        )
        .unwrap();
        create(
            &mut s.conn,
            "r.s1",
            State::Open,
            &[f("blocked-by", "[s0]")],
            "t",
        )
        .unwrap();
        create(&mut s.conn, "r.s0", State::Done, &[], "t").unwrap();
        move_card(&mut s.conn, "r", State::InProgress, "t1").unwrap();
        move_card(&mut s.conn, "r.s1", State::InProgress, "t2").unwrap();
        assert_eq!(
            events(&s.conn, "r.s1").unwrap(),
            [("t2".into(), "moved=inprogress".into())]
        );
        let err = move_card(&mut s.conn, "r.s1", State::Done, "t3").unwrap_err();
        assert!(matches!(
            err,
            MoveError::Refused {
                reason: Refusal::Transition,
                ..
            }
        ));
        assert_eq!(state(&s.conn, "r.s1").unwrap(), Some(State::InProgress));
        assert_eq!(events(&s.conn, "r.s1").unwrap().len(), 1);
    }

    #[test]
    fn ut_two_writers_at_once_lose_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = Store::open(dir.path(), true).unwrap();
        create(&mut s.conn, "r", State::Open, &[], "t").unwrap();
        let writers: Vec<_> = (0..2)
            .map(|w| {
                let root = dir.path().to_path_buf();
                std::thread::spawn(move || {
                    let mut s = Store::open(&root, false).unwrap();
                    for i in 0..50 {
                        let text = vec!["--".to_owned(), format!("w{w} n{i}")];
                        log(&mut s.conn, "r", "note", &text, "t").unwrap();
                    }
                })
            })
            .collect();
        for w in writers {
            w.join().unwrap();
        }
        assert_eq!(events(&s.conn, "r").unwrap().len(), 100);
    }
}
