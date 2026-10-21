//! The task board in the state database: a parent card per run and a stage
//! card per plan stage, each in one state, with frontmatter fields and an
//! append-only event log.

use std::collections::HashSet;
use std::fmt::{self, Write as _};
use std::path::Path;

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use crate::card::typed::{Change, Name, Typed, Verdict};
use crate::card::vocab::{self, Context, Reject};
use crate::store::{Board, Reader, Writer};

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
    /// `<slug>.s<n>` is a stage card, anything else a parent card; a
    /// stage that is not a plain number is no stage.
    #[must_use]
    pub fn parse(id: &'a str) -> Self {
        if let Some((run, stage)) = id.rsplit_once(".s")
            && stage.bytes().all(|b| b.is_ascii_digit())
            && stage.parse::<u32>().is_ok()
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

    /// Whether the run part is a [`valid_slug`]; a parent card whose id
    /// reads as a stage (`fix.s2`) is caught by the parse itself.
    #[must_use]
    pub fn valid(&self) -> bool {
        valid_slug(self.run)
    }
}

/// The longest slug a run takes.
pub const SLUG_MAX: usize = 64;

/// Whether `s` can name a run: ASCII letters, digits, `-`, `_` and `.`,
/// starting with a letter or digit, at most [`SLUG_MAX`] bytes. A slug
/// becomes a directory under `.ferrule/runs/`, a branch name and a card id,
/// so it holds no slash, no `..`, does not end in `.` or `.lock`, and does
/// not end in `.s<n>`, which would read as a stage card.
#[must_use]
pub fn valid_slug(s: &str) -> bool {
    s.len() <= SLUG_MAX
        && s.bytes().next().is_some_and(|b| b.is_ascii_alphanumeric())
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
        && !s.contains("..")
        && !s.ends_with('.')
        && !s.to_ascii_lowercase().ends_with(".lock")
        && CardId::parse(s).stage.is_none()
}

/// The `hint=` of a refused slug.
pub const SLUG_HINT: &str = "a slug is letters, digits, -, _ and single dots, starting with a letter or digit, at most 64 bytes, never ending in .s<n>";

/// A run's slug, checked by [`valid_slug`]; commands parse their slug
/// arguments into one so no path or branch is built from another.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Slug(String);

impl Slug {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::str::FromStr for Slug {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if valid_slug(s) {
            Ok(Slug(s.to_owned()))
        } else {
            Err(SLUG_HINT.to_owned())
        }
    }
}

impl std::ops::Deref for Slug {
    type Target = str;

    fn deref(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for Slug {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl AsRef<Path> for Slug {
    fn as_ref(&self) -> &Path {
        Path::new(&self.0)
    }
}

impl fmt::Display for Slug {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A card id argument: a [`valid_slug`], or one followed by `.s<n>`.
pub fn parse_card_id(s: &str) -> Result<String, String> {
    if CardId::parse(s).valid() {
        Ok(s.to_owned())
    } else {
        Err(format!("a card id is <slug> or <slug>.s<n>; {SLUG_HINT}"))
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
    "gate3",
    "gate4",
    "split",
    "upstream",
    "artifacts",
    "worktree",
];
/// Frontmatter keys a stage card may carry; `id` and `parent` are derived.
pub const STAGE_FIELDS: &[&str] = &["title", "blocked-by", "files", "branch", "worktree", "stop"];

/// The `hint=` of a refused field: the keys a stage or a parent card
/// takes.
#[must_use]
pub fn fields_hint(stage: bool) -> String {
    if stage {
        format!("a stage card takes only {}", STAGE_FIELDS.join(", "))
    } else {
        format!("a parent card takes only {}", PARENT_FIELDS.join(", "))
    }
}

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

/// Whether `s` is a stamp as the log writes it, `YYYY-MM-DDTHH:MM`.
#[must_use]
pub fn valid_stamp(s: &str) -> bool {
    let b = s.as_bytes();
    let num = |from: usize, to: usize| -> Option<u32> {
        let digits = b.get(from..to)?;
        digits
            .iter()
            .all(u8::is_ascii_digit)
            .then(|| digits.iter().fold(0, |n, d| n * 10 + u32::from(d - b'0')))
    };
    b.len() == 16
        && b[4] == b'-'
        && b[7] == b'-'
        && b[10] == b'T'
        && b[13] == b':'
        && num(0, 4).is_some()
        && num(5, 7).is_some_and(|m| (1..=12).contains(&m))
        && num(8, 10).is_some_and(|d| (1..=31).contains(&d))
        && num(11, 13).is_some_and(|h| h < 24)
        && num(14, 16).is_some_and(|m| m < 60)
}

/// The problem with `FERRULE_NOW`, when it is set and not a stamp.
#[must_use]
pub fn now_problem() -> Option<String> {
    let now = std::env::var("FERRULE_NOW").ok()?;
    (!valid_stamp(&now)).then(|| format!("FERRULE_NOW={now} is not a stamp like 2026-01-02T14:02"))
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

/// Whether `v` starts with a `YYYY-` date.
#[must_use]
pub fn is_date(v: Option<&str>) -> bool {
    v.is_some_and(|v| {
        let b = v.as_bytes();
        b.len() >= 5 && b[..4].iter().all(u8::is_ascii_digit) && b[4] == b'-'
    })
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

/// One log line as stored: its event name (through `=` when valued), the
/// value after that `=`, its args and its free text. Readers decide from
/// these, never from the rendered body, where a note's text could pass
/// for an event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    pub id: i64,
    pub card: String,
    pub at: String,
    pub name: String,
    pub value: Option<String>,
    /// `(key, value)` in logged order; a positional has no key.
    pub args: Vec<(Option<String>, String)>,
    pub text: Option<String>,
}

impl Event {
    /// The value of the line's last `key=`.
    #[must_use]
    pub fn arg(&self, key: &str) -> Option<&str> {
        self.args
            .iter()
            .rev()
            .find(|(k, _)| k.as_deref() == Some(key))
            .map(|(_, v)| v.as_str())
    }

    /// Positional word `n`.
    #[must_use]
    pub fn positional(&self, n: usize) -> Option<&str> {
        self.args
            .iter()
            .filter(|(k, _)| k.is_none())
            .nth(n)
            .map(|(_, v)| v.as_str())
    }

    /// `body` parsed back as a log line of card `r`.
    #[cfg(test)]
    pub(crate) fn parsed(body: &str) -> Self {
        let (name, parts) = vocab::parse(body);
        Event {
            id: 0,
            card: "r".to_owned(),
            at: "2026-10-04T11:50".to_owned(),
            name: name.to_owned(),
            value: parts.value,
            args: parts
                .args
                .into_iter()
                .map(|a| (a.key.map(str::to_owned), a.value))
                .collect(),
            text: parts.text,
        }
    }
}

/// Whose log lines [`select`] reads.
enum Of<'a> {
    Card(&'a str),
    Run(&'a str),
}

/// The log lines of `of` that are one of `names` (every line when empty),
/// in the order they were logged; only the newest when `last`.
fn select(conn: &Connection, of: &Of, names: &[Name], last: bool) -> rusqlite::Result<Vec<Event>> {
    let (scope, key) = match of {
        Of::Card(id) => ("card = ?1", *id),
        Of::Run(run) => ("card IN (SELECT id FROM cards WHERE run = ?1)", *run),
    };
    let id = if last { "max(id)" } else { "id" };
    let mut filter = format!("SELECT {id} FROM events WHERE {scope}");
    if !names.is_empty() {
        let marks: Vec<String> = (2..names.len() + 2).map(|i| format!("?{i}")).collect();
        let _ = write!(filter, " AND name IN ({})", marks.join(", "));
    }
    let mut st = conn.prepare(&format!(
        "SELECT e.id, e.card, e.at, e.name, e.value, e.text, a.key, a.value
         FROM events e LEFT JOIN event_args a ON a.event = e.id
         WHERE e.id IN ({filter})
         ORDER BY e.id, a.idx"
    ))?;
    let params = std::iter::once(key).chain(names.iter().map(|n| n.stored()));
    let mut rows = st.query(rusqlite::params_from_iter(params))?;
    let mut out: Vec<Event> = Vec::new();
    while let Some(r) = rows.next()? {
        let id: i64 = r.get(0)?;
        if out.last().is_none_or(|e| e.id != id) {
            out.push(Event {
                id,
                card: r.get(1)?,
                at: r.get(2)?,
                name: r.get(3)?,
                value: r.get(4)?,
                args: Vec::new(),
                text: r.get(5)?,
            });
        }
        if let (Some(e), Some(value)) = (out.last_mut(), r.get::<_, Option<String>>(7)?) {
            e.args.push((r.get(6)?, value));
        }
    }
    Ok(out)
}

/// A card's log lines that are one of `names` (every line when empty),
/// oldest first.
pub fn events(conn: &Connection, id: &str, names: &[Name]) -> rusqlite::Result<Vec<Event>> {
    select(conn, &Of::Card(id), names, false)
}

/// The log lines of every card of run `run` that are one of `names`
/// (every line when empty), in the order they were logged.
pub fn run_events(conn: &Connection, run: &str, names: &[Name]) -> rusqlite::Result<Vec<Event>> {
    select(conn, &Of::Run(run), names, false)
}

/// A card's newest log line that is one of `names`.
pub fn last_event(conn: &Connection, id: &str, names: &[Name]) -> rusqlite::Result<Option<Event>> {
    Ok(select(conn, &Of::Card(id), names, true)?.pop())
}

/// The value of a card's newest `name` line.
pub fn last_value(conn: &Connection, id: &str, name: Name) -> rusqlite::Result<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT value FROM events WHERE card = ?1 AND name = ?2 ORDER BY id DESC LIMIT 1",
            [id, name.stored()],
            |r| r.get::<_, Option<String>>(0),
        )
        .optional()?
        .flatten())
}

/// The value a card last logged under `key`: a `<key>=` event's own value
/// or a `key=` on any line, whichever is newer; free text never counts.
pub fn last_logged(conn: &Connection, id: &str, key: &str) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT v FROM (
             SELECT id, -1 AS idx, value AS v FROM events
             WHERE card = ?1 AND name = ?2 || '=' AND value IS NOT NULL
             UNION ALL
             SELECT e.id, a.idx, a.value FROM events e JOIN event_args a ON a.event = e.id
             WHERE e.card = ?1 AND a.key = ?2
         )
         ORDER BY id DESC, idx DESC LIMIT 1",
        [id, key],
        |r| r.get(0),
    )
    .optional()
}

/// The `sha=` of every `gauntlet=pass` line on a card, oldest first.
pub fn pass_shas(conn: &Connection, id: &str) -> rusqlite::Result<Vec<String>> {
    Ok(events(conn, id, &[Name::Gauntlet])?
        .iter()
        .filter_map(|e| match e.typed() {
            Typed::Gauntlet {
                result: Verdict::Pass,
                sha: Some(sha),
                ..
            } => Some(sha.to_owned()),
            _ => None,
        })
        .collect())
}

/// IDs of run `run`'s requirements reworded after filing, from the
/// `spec-delta <ID> changed origin=…` lines on any of its cards.
pub fn reworded_ids(conn: &Connection, run: &str) -> rusqlite::Result<HashSet<String>> {
    Ok(run_events(conn, run, &[Name::SpecDelta])?
        .iter()
        .filter_map(|e| match e.typed() {
            Typed::SpecDelta {
                id,
                change: Some(Change::Changed),
                origin: Some(_),
                ..
            } => Some(id.to_owned()),
            _ => None,
        })
        .collect())
}

/// The SQL condition that log line `e` is event `?2`, either written with
/// or without its `=`; every line when `?2` is null.
const NAMED: &str = "(?2 IS NULL OR rtrim(e.name, '=') = rtrim(?2, '='))";

/// A card's log lines as `(at, body)` for display, oldest first; `event`
/// (with or without its `=`) keeps that event's lines.
pub fn log_lines(
    conn: &Connection,
    id: &str,
    event: Option<&str>,
) -> rusqlite::Result<Vec<(String, String)>> {
    let mut st = conn.prepare(&format!(
        "SELECT e.at, e.body FROM events e WHERE e.card = ?1 AND {NAMED} ORDER BY e.id"
    ))?;
    st.query_map(params![id, event], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect()
}

/// Every log line of run `run`'s cards as `(card, at, body)` for display,
/// in the order they were logged; `event` (with or without its `=`) keeps
/// that event's lines.
pub fn run_log_lines(
    conn: &Connection,
    run: &str,
    event: Option<&str>,
) -> rusqlite::Result<Vec<(String, String, String)>> {
    let mut st = conn.prepare(&format!(
        "SELECT e.card, e.at, e.body FROM events e JOIN cards c ON c.id = e.card
         WHERE c.run = ?1 AND {NAMED} ORDER BY e.id"
    ))?;
    st.query_map(params![run, event], |r| {
        Ok((r.get(0)?, r.get(1)?, r.get(2)?))
    })?
    .collect()
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
    for (at, body) in log_lines(conn, id, None)? {
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
    /// An id whose run part is no [`valid_slug`].
    Id,
    Sql(rusqlite::Error),
}

impl fmt::Display for CardError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CardError::Card => f.write_str("no such card"),
            CardError::Id => write!(f, "not a card id: {SLUG_HINT}"),
            CardError::Field(k) => write!(f, "the card's kind does not carry {k}"),
            CardError::Exists => f.write_str("the card exists already"),
            CardError::Sql(e) => write!(f, "state database: {e}"),
        }
    }
}

impl std::error::Error for CardError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            CardError::Sql(e) => Some(e),
            CardError::Card | CardError::Field(_) | CardError::Exists | CardError::Id => None,
        }
    }
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
    Move(String, MoveError),
}

impl fmt::Display for BatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BatchError::Card(id, e) if id.is_empty() => write!(f, "{e}"),
            BatchError::Card(id, e) => write!(f, "card {id}: {e}"),
            BatchError::Log(id, e) => write!(f, "card {id}: {e}"),
            BatchError::Move(id, e) => write!(f, "card {id}: {e}"),
        }
    }
}

impl std::error::Error for BatchError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            BatchError::Card(_, e) => Some(e),
            BatchError::Log(_, e) => Some(e),
            BatchError::Move(_, e) => Some(e),
        }
    }
}

/// Creates every card and logs its lines, all or nothing.
pub fn create_all(conn: &mut Connection, cards: &[NewCard], now: &str) -> Result<(), BatchError> {
    let sql = |e: rusqlite::Error| BatchError::Card(String::new(), CardError::Sql(e));
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(sql)?;
    insert_all(&tx, cards, now)?;
    tx.commit().map_err(sql)
}

/// Sets `fields` on `id`, moves it to `to` (refused as `move_card` refuses)
/// and creates `create`, all or nothing: a refused move writes no field
/// and no card.
pub fn set_move_create(
    conn: &mut Connection,
    id: &str,
    fields: &[(String, String)],
    to: State,
    create: &[NewCard],
    now: &str,
) -> Result<(), BatchError> {
    let sql = |e: rusqlite::Error| BatchError::Card(String::new(), CardError::Sql(e));
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(sql)?;
    set_in(&tx, id, fields).map_err(|e| BatchError::Card(id.to_owned(), e))?;
    move_in(&tx, id, to, now).map_err(|e| BatchError::Move(id.to_owned(), e))?;
    insert_all(&tx, create, now)?;
    tx.commit().map_err(sql)
}

fn insert_all(conn: &Connection, cards: &[NewCard], now: &str) -> Result<(), BatchError> {
    for c in cards {
        insert(conn, &c.id, c.state, &c.fields, now)
            .map_err(|e| BatchError::Card(c.id.clone(), e))?;
        for (event, args) in &c.log {
            append(conn, &c.id, event, args, now, vocab::READ_BUDGET, false)
                .map_err(|e| BatchError::Log(c.id.clone(), e))?;
        }
    }
    Ok(())
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
        append(&tx, parent, event, args, now, vocab::READ_BUDGET, false)
            .map_err(|e| BatchError::Log(parent.to_owned(), e))?;
    }
    for id in delete {
        tx.execute("DELETE FROM cards WHERE id = ?1", [id])
            .map_err(sql)?;
    }
    insert_all(&tx, create, now)?;
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
    if !card.valid() {
        return Err(CardError::Id);
    }
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
    set_all(conn, id, &[(key.to_owned(), value.to_owned())])
}

/// Sets frontmatter fields in one transaction: every key is checked first,
/// and nothing is written when one is refused. An empty value removes a
/// field.
pub fn set_all(
    conn: &mut Connection,
    id: &str,
    fields: &[(String, String)],
) -> Result<(), CardError> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    set_in(&tx, id, fields)?;
    tx.commit()?;
    Ok(())
}

fn set_in(tx: &Connection, id: &str, fields: &[(String, String)]) -> Result<(), CardError> {
    let card = CardId::parse(id);
    if let Some((k, _)) = fields.iter().find(|(k, _)| !allowed(&card, k)) {
        return Err(CardError::Field(k.clone()));
    }
    if state(tx, id)?.is_none() {
        return Err(CardError::Card);
    }
    for (key, value) in fields {
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
    }
    Ok(())
}

#[derive(Debug)]
pub enum LogError {
    Card,
    Reject(Reject),
    Sql(rusqlite::Error),
}

impl fmt::Display for LogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LogError::Card => f.write_str("no such card"),
            LogError::Reject(r) => write!(f, "{r}"),
            LogError::Sql(e) => write!(f, "state database: {e}"),
        }
    }
}

impl std::error::Error for LogError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            LogError::Reject(r) => Some(r),
            LogError::Sql(e) => Some(e),
            LogError::Card => None,
        }
    }
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
    log_with_budget(conn, id, event, args, now, vocab::READ_BUDGET, false)
}

/// [`log`] with the project's `read` budget; `agent` is a line from
/// `ferrule board log`, which may not carry an event only ferrule writes.
pub fn log_with_budget(
    conn: &mut Connection,
    id: &str,
    event: &str,
    args: &[String],
    now: &str,
    read_budget: usize,
    agent: bool,
) -> Result<(), LogError> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    append(&tx, id, event, args, now, read_budget, agent)?;
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
    agent: bool,
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
        agent,
    };
    let line = vocab::check(event, args, ctx).map_err(LogError::Reject)?;
    if now.chars().count() + 1 + line.body.chars().count() > vocab::MAX_LINE {
        return Err(LogError::Reject(Reject {
            event: line.name.trim_end_matches('=').to_owned(),
            reason: vocab::Reason::Length,
            spec: vocab::lookup(line.name),
        }));
    }
    insert_event(conn, id, now, line.name, &line.parts, &line.body)?;
    Ok(())
}

/// Appends one log line with its value, args and text.
fn insert_event(
    conn: &Connection,
    id: &str,
    at: &str,
    name: &str,
    parts: &vocab::Parts,
    body: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO events (card, at, name, body, value, text) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![id, at, name, body, parts.value, parts.text],
    )?;
    insert_args(conn, conn.last_insert_rowid(), &parts.args)
}

fn insert_args(conn: &Connection, event: i64, args: &[vocab::Arg]) -> rusqlite::Result<()> {
    let mut st = conn.prepare_cached(
        "INSERT INTO event_args (event, idx, key, value) VALUES (?1, ?2, ?3, ?4)",
    )?;
    for (idx, a) in args.iter().enumerate() {
        st.execute(params![event, idx, a.key, a.value])?;
    }
    Ok(())
}

/// Stores `body` on card `id` unchecked, as an import or the backfill
/// does: parsed back, its keys in the order written.
#[cfg(test)]
pub(crate) fn insert_body(conn: &Connection, id: &str, at: &str, body: &str) {
    let (name, parts) = vocab::parse(body);
    insert_event(conn, id, at, name, &parts, body).unwrap();
}

/// Fills the value, args and text of log lines stored before they were
/// kept apart, by parsing each body back.
pub(crate) fn backfill_args(conn: &Connection) -> rusqlite::Result<()> {
    let rows: Vec<(i64, String)> = {
        let mut st = conn.prepare("SELECT id, body FROM events ORDER BY id")?;
        st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?
    };
    for (event, body) in rows {
        let (_, parts) = vocab::parse(&body);
        conn.execute(
            "UPDATE events SET value = ?1, text = ?2 WHERE id = ?3",
            params![parts.value, parts.text, event],
        )?;
        insert_args(conn, event, &parts.args)?;
    }
    Ok(())
}

/// A log line's args as `(key, value)` in order; a positional has no key.
pub fn event_args(
    conn: &Connection,
    event: i64,
) -> rusqlite::Result<Vec<(Option<String>, String)>> {
    let mut st = conn.prepare("SELECT key, value FROM event_args WHERE event = ?1 ORDER BY idx")?;
    st.query_map([event], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect()
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
    pub fn hint(self, from: State, stage: bool) -> String {
        match self {
            Refusal::Transition => {
                let to = if stage {
                    match from {
                        State::Open | State::Done => "inprogress",
                        State::InProgress => "inreview",
                        State::InReview => "done or inprogress",
                    }
                } else {
                    match from {
                        State::Open => "inprogress (after gate2), or inreview for an epic",
                        State::InProgress => "inreview",
                        State::InReview => "inprogress",
                        State::Done => "nowhere; a done parent stays done",
                    }
                };
                let kind = if stage { "stage" } else { "parent" };
                format!("from {from} a {kind} card can move only to {to}")
            }
            Refusal::BlockedBy => "move the cards in its blocked-by to done first".to_owned(),
            Refusal::Commit => {
                "commit the stage with `ferrule stage commit <card> -m <msg>` first".to_owned()
            }
            Refusal::Unverified => {
                "run `ferrule ci gauntlet <slug>`, then `ferrule pr push <slug>`, which logs the gauntlet=pass that covers the commit sha"
                    .to_owned()
            }
            Refusal::Gate2 => {
                "approve gate 2 first: `ferrule gate approve <slug> gate2`".to_owned()
            }
        }
    }
}

#[derive(Debug)]
pub enum MoveError {
    Card,
    /// The `moved=` line was refused (it is over the line length).
    Log(Reject),
    Refused {
        from: State,
        reason: Refusal,
    },
    Sql(rusqlite::Error),
}

impl MoveError {
    /// The refusal line of moving card `id` to `to`; `None` for a state
    /// database error.
    #[must_use]
    pub fn refusal(&self, id: &str, to: State) -> Option<crate::refusal::Refusal> {
        match self {
            MoveError::Card => Some(crate::refusal::Refusal::card("card-move")),
            MoveError::Log(r) => Some(r.refusal()),
            MoveError::Refused { from, reason } => Some(
                crate::refusal::Refusal::new("card-move", reason.as_str())
                    .field("from", from)
                    .field("to", to)
                    .hint(reason.hint(*from, CardId::parse(id).stage.is_some()))
                    .exit(crate::refusal::Exit::Usage),
            ),
            MoveError::Sql(_) => None,
        }
    }
}

impl fmt::Display for MoveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MoveError::Card => f.write_str("no such card"),
            MoveError::Log(r) => write!(f, "{r}"),
            MoveError::Refused { from, reason } => {
                write!(f, "move from {from} refused: {}", reason.as_str())
            }
            MoveError::Sql(e) => write!(f, "state database: {e}"),
        }
    }
}

impl std::error::Error for MoveError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            MoveError::Log(r) => Some(r),
            MoveError::Sql(e) => Some(e),
            MoveError::Card | MoveError::Refused { .. } => None,
        }
    }
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
    move_in(&tx, id, to, now)?;
    tx.commit()?;
    Ok(())
}

fn move_in(tx: &Connection, id: &str, to: State, now: &str) -> Result<(), MoveError> {
    let Some(from) = state(tx, id)? else {
        return Err(MoveError::Card);
    };
    if from == to {
        return Ok(());
    }
    if let Some(reason) = refusal(tx, &CardId::parse(id), from, to)? {
        return Err(MoveError::Refused { from, reason });
    }
    tx.execute(
        "UPDATE cards SET state = ?1 WHERE id = ?2",
        [to.as_str(), id],
    )?;
    match append(
        tx,
        id,
        &format!("moved={to}"),
        &[],
        now,
        vocab::READ_BUDGET,
        false,
    ) {
        Ok(()) => {}
        Err(LogError::Sql(e)) => return Err(MoveError::Sql(e)),
        Err(LogError::Card) => return Err(MoveError::Card),
        Err(LogError::Reject(r)) => return Err(MoveError::Log(r)),
    }
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
            (InReview, Done) => match last_value(conn, card.id, Name::Commit)? {
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

impl std::error::Error for ImportError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ImportError::Io(e) => Some(e),
            ImportError::Sql(e) => Some(e),
            ImportError::Card { .. } | ImportError::Exists(_) => None,
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
                "state" if State::parse(&v).is_none() => {
                    return Err(bad(format!("state: {v} is not a card state")));
                }
                "state" if State::parse(&v) != Some(*st) => {
                    return Err(bad(format!("state: {v} differs from the {st} directory")));
                }
                "id" | "parent" | "state" => {}
                _ => fields.push((k, v)),
            }
        }
        match insert(&tx, id, *st, &fields, now) {
            Ok(()) => {}
            Err(CardError::Exists) => return Err(ImportError::Exists(id.to_owned())),
            Err(CardError::Field(k)) => {
                return Err(bad(format!("unknown frontmatter key {k}")));
            }
            Err(CardError::Id) => return Err(bad(format!("{id} is not a card id: {SLUG_HINT}"))),
            Err(CardError::Sql(e)) => return Err(ImportError::Sql(e)),
            Err(CardError::Card) => unreachable!("insert never looks a card up by id"),
        }
        for (at, body) in card.events {
            if !valid_stamp(&at) {
                return Err(bad(format!(
                    "log stamp {at} is not a stamp like 2026-01-02T14:02"
                )));
            }
            let (name, parts) = vocab::parse(&body);
            insert_event(&tx, id, &at, name, &parts, &body)?;
        }
    }
    tx.commit()?;
    Ok(files.len())
}

/// The board reads, over a snapshot; each is the free function of the
/// same name.
impl Reader<'_> {
    pub fn state(&self, id: &str) -> rusqlite::Result<Option<State>> {
        state(self.conn(), id)
    }

    pub fn field(&self, id: &str, key: &str) -> rusqlite::Result<Option<String>> {
        field(self.conn(), id, key)
    }

    pub fn fields(&self, id: &str) -> rusqlite::Result<Vec<(String, String)>> {
        fields(self.conn(), id)
    }

    pub fn events(&self, id: &str, names: &[Name]) -> rusqlite::Result<Vec<Event>> {
        events(self.conn(), id, names)
    }

    pub fn run_events(&self, run: &str, names: &[Name]) -> rusqlite::Result<Vec<Event>> {
        run_events(self.conn(), run, names)
    }

    pub fn last_logged(&self, id: &str, key: &str) -> rusqlite::Result<Option<String>> {
        last_logged(self.conn(), id, key)
    }

    pub fn pass_shas(&self, id: &str) -> rusqlite::Result<Vec<String>> {
        pass_shas(self.conn(), id)
    }

    pub fn reworded_ids(&self, run: &str) -> rusqlite::Result<HashSet<String>> {
        reworded_ids(self.conn(), run)
    }

    pub fn log_lines(
        &self,
        id: &str,
        event: Option<&str>,
    ) -> rusqlite::Result<Vec<(String, String)>> {
        log_lines(self.conn(), id, event)
    }

    pub fn run_log_lines(
        &self,
        run: &str,
        event: Option<&str>,
    ) -> rusqlite::Result<Vec<(String, String, String)>> {
        run_log_lines(self.conn(), run, event)
    }

    pub fn show(&self, id: &str) -> rusqlite::Result<Option<String>> {
        show(self.conn(), id)
    }

    pub fn all(&self) -> rusqlite::Result<Vec<(String, State)>> {
        all(self.conn())
    }
}

/// The board writes, inside the writer's transaction; each is the free
/// function of the same name.
impl Writer<'_> {
    pub fn create(
        &self,
        id: &str,
        state: State,
        fields: &[(String, String)],
        now: &str,
    ) -> Result<(), CardError> {
        insert(self.conn(), id, state, fields, now)
    }

    pub fn set_all(&self, id: &str, fields: &[(String, String)]) -> Result<(), CardError> {
        set_in(self.conn(), id, fields)
    }

    pub fn log(&self, id: &str, event: &str, args: &[String], now: &str) -> Result<(), LogError> {
        self.log_with_budget(id, event, args, now, vocab::READ_BUDGET, false)
    }

    pub fn log_with_budget(
        &self,
        id: &str,
        event: &str,
        args: &[String],
        now: &str,
        read_budget: usize,
        agent: bool,
    ) -> Result<(), LogError> {
        append(self.conn(), id, event, args, now, read_budget, agent)
    }

    pub fn move_card(&self, id: &str, to: State, now: &str) -> Result<(), MoveError> {
        move_in(self.conn(), id, to, now)
    }
}

impl Board {
    /// [`import`] into this board: one transaction of its own around the
    /// file reads.
    pub fn import(&mut self, board: &Path, now: &str) -> Result<usize, ImportError> {
        import(self.conn_mut(), board, now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::store::Store;

    fn raw(s: &Store, card: &str, at: &str, body: &str) {
        insert_body(&s.conn, card, at, body);
    }

    #[test]
    fn ut_slug_is_one_plain_path_segment() {
        for ok in [
            "demo",
            "add-backoff",
            "a_1",
            "7",
            "my.slug",
            &"x".repeat(SLUG_MAX),
        ] {
            assert!(valid_slug(ok), "{ok}");
        }
        for bad in [
            "",
            ".",
            "..",
            "../x",
            "a/b",
            "a..b",
            "a.",
            "a.lock",
            "-x",
            "_x",
            "a b",
            "fix.s2",
            "é",
            &"x".repeat(SLUG_MAX + 1),
        ] {
            assert!(!valid_slug(bad), "{bad}");
        }
        assert!(parse_card_id("demo.s3").is_ok());
        assert!(parse_card_id("../x.s3").is_err());
        let mut s = store();
        assert!(matches!(
            create(&mut s.conn, "../x", State::Open, &[], "t"),
            Err(CardError::Id)
        ));
    }

    #[test]
    fn ut_reworded_needs_origin_among_leading_keys() {
        let mut s = store();
        create(&mut s.conn, "r", State::Open, &[], "t").unwrap();
        create(&mut s.conn, "r.s1", State::Open, &[], "t").unwrap();
        raw(
            &s,
            "r",
            "t",
            "spec-delta MB-R-294 changed origin=user popup",
        );
        raw(
            &s,
            "r.s1",
            "t",
            "spec-delta MB-R-301 changed quote=\"a b\" origin=user box",
        );
        raw(
            &s,
            "r",
            "t",
            "spec-delta MB-R-302 changed popup origin=user",
        );
        raw(&s, "r", "t", "spec-delta MB-R-303 added origin=user");
        raw(&s, "r", "t", "note spec-delta MB-R-304 changed origin=user");
        let mut ids: Vec<String> = reworded_ids(&s.conn, "r").unwrap().into_iter().collect();
        ids.sort();
        assert_eq!(ids, ["MB-R-294", "MB-R-301"]);
    }

    #[test]
    fn ut_card_ids() {
        assert_eq!(CardId::parse("demo.s3").stage, Some("3"));
        assert_eq!(CardId::parse("demo.s3").run, "demo");
        assert_eq!(CardId::parse("demo").stage, None);
        assert_eq!(CardId::parse("my.slug").stage, None);
        assert_eq!(CardId::parse("a.stuff").stage, None);
        assert_eq!(CardId::parse("my.slug.s1").run, "my.slug");
        assert_eq!(CardId::parse("my.slug.s1").stage, Some("1"));
        assert_eq!(CardId::parse("r.s1x").stage, None);
        assert_eq!(CardId::parse("r.s1x").run, "r.s1x");
        assert_eq!(CardId::parse("r.s+1").stage, None);
        assert_eq!(CardId::parse("r.s99999999999").stage, None);
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
            log_lines(&s.conn, "r.s1", None).unwrap(),
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
        assert_eq!(events(&s.conn, "r.s1", &[]).unwrap().len(), 1);
    }

    #[test]
    fn ut_a_refused_move_writes_no_field_and_no_card() {
        let mut s = store();
        let f = |k: &str, v: &str| (k.to_owned(), v.to_owned());
        create(&mut s.conn, "r", State::Done, &[f("lane", "full")], "t").unwrap();
        let card = NewCard {
            id: "r.s0".to_owned(),
            state: State::Open,
            fields: vec![f("title", "Land spec")],
            log: Vec::new(),
        };
        let set = [f("gate2", "2026-10-03"), f("branch", "feat/r")];
        let err = set_move_create(
            &mut s.conn,
            "r",
            &set,
            State::InProgress,
            std::slice::from_ref(&card),
            "t1",
        )
        .unwrap_err();
        assert!(matches!(
            err,
            BatchError::Move(
                _,
                MoveError::Refused {
                    reason: Refusal::Transition,
                    ..
                }
            )
        ));
        assert_eq!(fields(&s.conn, "r").unwrap(), [f("lane", "full")]);
        assert!(stages(&s.conn, "r").unwrap().is_empty());
        assert!(events(&s.conn, "r", &[]).unwrap().is_empty());

        // From open, the gate2 written in the same transaction lets it move.
        create(&mut s.conn, "q", State::Open, &[], "t").unwrap();
        let card = NewCard {
            id: "q.s0".to_owned(),
            ..card
        };
        set_move_create(&mut s.conn, "q", &set, State::InProgress, &[card], "t1").unwrap();
        assert_eq!(state(&s.conn, "q").unwrap(), Some(State::InProgress));
        assert_eq!(
            field(&s.conn, "q", "gate2").unwrap().as_deref(),
            Some("2026-10-03")
        );
        assert_eq!(stages(&s.conn, "q").unwrap().len(), 1);
    }

    #[test]
    fn ut_log_stores_args_and_text_apart() {
        let mut s = store();
        create(&mut s.conn, "r", State::Open, &[], "t").unwrap();
        create(&mut s.conn, "r.s1", State::Open, &[], "t").unwrap();
        let a = |s: &str| s.split(' ').map(str::to_owned).collect::<Vec<_>>();
        log(
            &mut s.conn,
            "r",
            "stop",
            &a("gate=gate2 kind=approval"),
            "t",
        )
        .unwrap();
        let reason = vec!["user=dl".to_owned(), "reason=a b".to_owned()];
        log(&mut s.conn, "r.s1", "files+=src/x.rs", &reason, "t").unwrap();
        log(&mut s.conn, "r.s1", "note", &a("-- then commit=zzz"), "t").unwrap();
        let rows: Vec<(i64, String, Option<String>, Option<String>)> = s
            .conn
            .prepare("SELECT id, body, value, text FROM events ORDER BY id")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        let kv = |k: &str, v: &str| (Some(k.to_owned()), v.to_owned());

        assert_eq!(rows[0].1, "stop kind=approval gate=gate2");
        assert_eq!(
            event_args(&s.conn, rows[0].0).unwrap(),
            [kv("kind", "approval"), kv("gate", "gate2")]
        );
        assert_eq!(rows[1].1, "files+=src/x.rs user=dl reason=\"a b\"");
        assert_eq!(rows[1].2.as_deref(), Some("src/x.rs"));
        assert_eq!(
            event_args(&s.conn, rows[1].0).unwrap(),
            [kv("user", "dl"), kv("reason", "a b")]
        );
        assert_eq!(rows[2].3.as_deref(), Some("then commit=zzz"));
        assert!(event_args(&s.conn, rows[2].0).unwrap().is_empty());
    }

    #[test]
    fn ut_note_text_is_no_evidence() {
        let mut s = store();
        let gate2 = [("gate2".to_owned(), "2026-10-03".to_owned())];
        create(&mut s.conn, "r", State::InProgress, &gate2, "t").unwrap();
        create(&mut s.conn, "r.s1", State::InReview, &[], "t").unwrap();
        let a = |s: &str| s.split(' ').map(str::to_owned).collect::<Vec<_>>();
        log(&mut s.conn, "r.s1", "commit=abc", &[], "t").unwrap();
        log(
            &mut s.conn,
            "r",
            "note",
            &a("-- saw gauntlet=pass sha=abc"),
            "t",
        )
        .unwrap();
        log(&mut s.conn, "r.s1", "note", &a("-- then commit=zzz"), "t").unwrap();

        assert!(pass_shas(&s.conn, "r").unwrap().is_empty());
        assert_eq!(
            last_logged(&s.conn, "r.s1", "commit").unwrap().as_deref(),
            Some("abc")
        );
        assert!(matches!(
            move_card(&mut s.conn, "r.s1", State::Done, "t"),
            Err(MoveError::Refused {
                reason: Refusal::Unverified,
                ..
            })
        ));
        log(&mut s.conn, "r", "gauntlet=pass", &a("sha=abc"), "t").unwrap();
        assert_eq!(pass_shas(&s.conn, "r").unwrap(), ["abc"]);
        move_card(&mut s.conn, "r.s1", State::Done, "t").unwrap();
    }

    #[test]
    fn ut_readers_take_args_by_key_unquoted() {
        let mut s = store();
        create(&mut s.conn, "r", State::Open, &[], "t").unwrap();
        create(&mut s.conn, "r.s1", State::Open, &[], "t").unwrap();
        let reason = vec!["reason=a b".to_owned(), "user=dl".to_owned()];
        log(&mut s.conn, "r.s1", "files+=src/x.rs", &reason, "t").unwrap();
        raw(&s, "r", "t", "stop gate=gate4 kind=approval");

        assert_eq!(
            last_logged(&s.conn, "r.s1", "reason").unwrap().as_deref(),
            Some("a b")
        );
        let files = last_event(&s.conn, "r.s1", &[]).unwrap().unwrap();
        assert_eq!(
            (files.value.as_deref(), files.arg("user")),
            (Some("src/x.rs"), Some("dl"))
        );
        let stop = last_event(&s.conn, "r", &[Name::Stop]).unwrap().unwrap();
        assert_eq!(
            stop.typed(),
            Typed::Stop {
                kind: Some("approval"),
                gate: Some("gate4")
            }
        );
        assert_eq!(
            last_event(&s.conn, "r", &[Name::ReplanApproved]).unwrap(),
            None
        );
    }

    #[test]
    fn ut_latest_in_a_minute_is_the_last_logged() {
        let mut s = store();
        create(&mut s.conn, "r", State::Open, &[], "t").unwrap();
        create(&mut s.conn, "r.s1", State::Open, &[], "t").unwrap();
        let note = |t: &str| vec!["--".to_owned(), t.to_owned()];
        let at = "2026-10-04T11:50";
        log(&mut s.conn, "r.s1", "note", &note("first"), at).unwrap();
        log(&mut s.conn, "r", "note", &note("second"), at).unwrap();
        log(&mut s.conn, "r", "ci=1", &[], at).unwrap();
        log(&mut s.conn, "r", "ci=0", &[], at).unwrap();

        let run: Vec<String> = run_events(&s.conn, "r", &[])
            .unwrap()
            .into_iter()
            .filter_map(|e| e.text)
            .collect();
        assert_eq!(run, ["first", "second"]);
        assert_eq!(
            run_log_lines(&s.conn, "r", None).unwrap().pop().unwrap().2,
            "ci=0"
        );
        assert_eq!(
            last_value(&s.conn, "r", Name::Ci).unwrap().as_deref(),
            Some("0")
        );
        assert_eq!(log_lines(&s.conn, "r", Some("ci")).unwrap().len(), 2);
    }

    #[test]
    fn ut_a_stamp_is_year_to_minute() {
        assert!(valid_stamp("2026-10-05T09:00"));
        for bad in [
            "",
            "t",
            "2026-10-05",
            "2026-10-05 09:00",
            "2026-10-05T09:00:00",
            "2026-13-05T09:00",
            "2026-10-05T24:00",
            "2026-10-05T09:60",
            "2026-1a-05T09:00",
        ] {
            assert!(!valid_stamp(bad), "{bad}");
        }
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
        assert_eq!(events(&s.conn, "r", &[]).unwrap().len(), 100);
    }
}
