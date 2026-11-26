//! The board as one JSON document: runs with their cards, events and the
//! stop a run waits on.

use serde::Serialize;

use crate::card::board::{self, Event, State};
use crate::card::typed::Typed;
use crate::card::vocab;
use crate::prompt::approval;
use crate::run::archive;
use crate::store::Reader;

pub const VERSION: u32 = 1;

#[derive(Serialize)]
pub struct Dump {
    pub version: u32,
    pub runs: Vec<RunEntry>,
}

#[derive(Serialize)]
pub struct RunEntry {
    pub slug: String,
    /// `None` for stage cards on the board without their run card, and for
    /// an archive without a parent card file.
    pub card: Option<CardEntry>,
    pub stages: Vec<CardEntry>,
    pub events: Vec<EventEntry>,
    pub waiting: Option<Waiting>,
    pub archived: Option<Archived>,
}

#[derive(Serialize)]
pub struct CardEntry {
    pub id: String,
    /// `s<n>` on a stage card; absent from the run card's JSON.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage: Option<String>,
    pub state: String,
    /// Serialized as a JSON object in stored order.
    pub fields: serde_json::Map<String, serde_json::Value>,
}

#[derive(Serialize)]
pub struct EventEntry {
    pub card: String,
    pub at: String,
    pub name: String,
    pub value: Option<String>,
    /// Keyed args in logged order; a repeated key keeps its last value.
    pub args: serde_json::Map<String, serde_json::Value>,
    pub positional: Vec<String>,
    pub text: Option<String>,
}

#[derive(Serialize)]
pub struct Waiting {
    pub kind: Option<String>,
    pub gate: Option<String>,
    pub at: String,
    pub files: Vec<String>,
}

#[derive(Serialize)]
pub struct Archived {
    pub at: String,
    pub abandoned: bool,
}

fn event_entry(e: &Event) -> EventEntry {
    let mut args = serde_json::Map::new();
    let mut positional = Vec::new();
    for (key, value) in &e.args {
        match key {
            Some(k) => {
                args.insert(k.clone(), serde_json::Value::String(value.clone()));
            }
            None => positional.push(value.clone()),
        }
    }
    EventEntry {
        card: e.card.clone(),
        at: e.at.clone(),
        name: e.name.strip_suffix('=').unwrap_or(&e.name).to_owned(),
        value: e.value.clone(),
        args,
        positional,
        text: e.text.clone(),
    }
}

/// The run's last `stop` when no later event answers it, matched through
/// `Event::typed()`: an approval stop with a gate by a later `approved=`
/// of that gate, without a gate by any later `approved=`; any other stop
/// by any later event.
#[must_use]
pub fn waiting(events: &[Event]) -> Option<&Event> {
    let (at, stop) = events
        .iter()
        .enumerate()
        .rev()
        .find(|(_, e)| matches!(e.typed(), Typed::Stop { .. }))?;
    let Typed::Stop { kind, gate } = stop.typed() else {
        return None;
    };
    let later = &events[at + 1..];
    let answered = if kind == Some("approval") {
        later.iter().any(|e| match (e.typed(), gate) {
            (Typed::Approved(v), Some(g)) => v == g,
            (Typed::Approved(_), None) => true,
            _ => false,
        })
    } else {
        !later.is_empty()
    };
    (!answered).then_some(stop)
}

fn fields_map(
    pairs: impl IntoIterator<Item = (String, String)>,
) -> serde_json::Map<String, serde_json::Value> {
    pairs
        .into_iter()
        .map(|(k, v)| (k, serde_json::Value::String(v)))
        .collect()
}

fn kept(card: Option<&CardEntry>, wanted: &[State]) -> bool {
    wanted.is_empty() || card.is_some_and(|c| wanted.iter().any(|s| s.as_str() == c.state))
}

fn live(
    r: &Reader<'_>,
    run: Option<&str>,
    wanted: &[State],
    out: &mut Vec<RunEntry>,
) -> Result<(), crate::run::archive::Error> {
    let all = r.all()?;
    let mut slugs: Vec<&str> = Vec::new();
    for (id, _) in &all {
        let slug = board::CardId::parse(id).run;
        if !slugs.contains(&slug) && run.is_none_or(|w| w == slug) {
            slugs.push(slug);
        }
    }
    for slug in slugs {
        let card = match all.iter().find(|(id, _)| id == slug) {
            Some((id, st)) => Some(CardEntry {
                id: id.clone(),
                stage: None,
                state: st.as_str().to_owned(),
                fields: fields_map(r.fields(id)?),
            }),
            None => None,
        };
        if !kept(card.as_ref(), wanted) {
            continue;
        }
        let mut stages = Vec::new();
        for (n, id, st) in board::stages(r.conn(), slug)? {
            stages.push(CardEntry {
                fields: fields_map(r.fields(&id)?),
                id,
                stage: Some(format!("s{n}")),
                state: st.as_str().to_owned(),
            });
        }
        let events = r.run_events(slug, &[])?;
        let waiting = match waiting(&events) {
            Some(stop) => {
                let (kind, gate) = match stop.typed() {
                    Typed::Stop { kind, gate } => {
                        (kind.map(str::to_owned), gate.map(str::to_owned))
                    }
                    _ => (None, None),
                };
                let files = approval::marker(r.conn(), slug)?
                    .into_iter()
                    .filter(|(at, _)| *at == stop.at)
                    .map(|(_, p)| p.to_string_lossy().into_owned())
                    .collect();
                Some(Waiting {
                    kind,
                    gate,
                    at: stop.at.clone(),
                    files,
                })
            }
            None => None,
        };
        out.push(RunEntry {
            slug: slug.to_owned(),
            card,
            stages,
            events: events.iter().map(event_entry).collect(),
            waiting,
            archived: None,
        });
    }
    Ok(())
}

/// A card file's entry: `id`, `state` and the fields after `parent`.
fn archived_card(id: &str, file: &board::CardFile) -> CardEntry {
    let get = |k: &str| {
        file.fields
            .iter()
            .find(|(f, _)| f == k)
            .map(|(_, v)| v.clone())
    };
    let stage = board::CardId::parse(id).stage.map(|n| format!("s{n}"));
    CardEntry {
        id: get("id").unwrap_or_else(|| id.to_owned()),
        stage,
        state: get("state").unwrap_or_default(),
        fields: fields_map(
            file.fields
                .iter()
                .filter(|(k, _)| !matches!(k.as_str(), "id" | "parent" | "state"))
                .cloned(),
        ),
    }
}

fn archived_event(card: &str, at: &str, body: &str) -> Event {
    let (name, parts) = vocab::parse(body);
    Event {
        id: 0,
        card: card.to_owned(),
        at: at.to_owned(),
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

fn archived(
    r: &Reader<'_>,
    run: Option<&str>,
    wanted: &[State],
    out: &mut Vec<RunEntry>,
) -> Result<(), crate::run::archive::Error> {
    for a in archive::archived_runs(r.conn(), run)? {
        let mut card = None;
        let mut stages = Vec::new();
        let mut events = Vec::new();
        for (id, file) in &a.cards {
            let entry = archived_card(id, file);
            if board::CardId::parse(id).stage.is_none() {
                card = Some(entry);
            } else {
                stages.push(entry);
            }
            events.extend(
                file.events
                    .iter()
                    .map(|(at, body)| event_entry(&archived_event(id, at, body))),
            );
        }
        if !kept(card.as_ref(), wanted) {
            continue;
        }
        events.sort_by(|x, y| x.at.cmp(&y.at));
        out.push(RunEntry {
            slug: a.run,
            card,
            stages,
            events,
            waiting: None,
            archived: Some(Archived {
                at: a.at,
                abandoned: a.abandoned,
            }),
        });
    }
    Ok(())
}

/// The board as a document: live runs in slug order, then, with
/// `archived`, each archive oldest first. `run` keeps one slug, `wanted`
/// (when not empty) the runs whose card is in one of them.
pub fn dump(
    r: &Reader<'_>,
    run: Option<&str>,
    wanted: &[State],
    archived: bool,
) -> Result<Dump, crate::run::archive::Error> {
    let mut runs = Vec::new();
    live(r, run, wanted, &mut runs)?;
    if archived {
        self::archived(r, run, wanted, &mut runs)?;
    }
    Ok(Dump {
        version: VERSION,
        runs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::card::board;
    use crate::run::archive::{CiCheck, Seal, seal};
    use crate::store::Store;

    const AT: &str = "2026-10-04T11:50";

    fn ev(body: &str) -> Event {
        Event::parsed(body)
    }

    /// BO-R-014 — an approval stop nothing answers is what the run waits on.
    #[test]
    fn ut_waiting_approval_unanswered() {
        let events = [ev("note x"), ev("stop kind=approval gate=gate2")];
        assert_eq!(waiting(&events).map(|e| e.name.as_str()), Some("stop"));
        assert!(waiting(&[ev("note x")]).is_none());
    }

    /// BO-R-015 — a later `approved=` equal to the stop's gate answers it, another gate does not.
    #[test]
    fn ut_waiting_approval_answered_by_its_gate() {
        let stop = "stop kind=approval gate=gate2";
        assert!(waiting(&[ev(stop), ev("approved=gate2")]).is_none());
        assert!(waiting(&[ev(stop), ev("approved=gate1")]).is_some());
    }

    /// BO-R-015 — the retired `delta` gate is answered by `approved=delta`.
    #[test]
    fn ut_waiting_delta_gate() {
        let stop = "stop kind=approval gate=delta";
        assert!(waiting(&[ev(stop), ev("approved=delta")]).is_none());
        assert!(waiting(&[ev(stop), ev("approved=gate2")]).is_some());
    }

    /// BO-E-012 — an approval stop without a gate is answered by any `approved=`.
    #[test]
    fn ut_waiting_approval_without_gate() {
        let stop = "stop kind=approval";
        assert!(waiting(&[ev(stop), ev("approved=gate9")]).is_none());
        assert!(waiting(&[ev(stop), ev("note x")]).is_some());
    }

    /// BO-R-016 — a non-approval stop, with or without a kind, is answered by any later event.
    #[test]
    fn ut_waiting_other_stop_answered_by_any_event() {
        for stop in ["stop kind=question", "stop"] {
            assert!(waiting(&[ev(stop)]).is_some(), "{stop}");
            assert!(waiting(&[ev(stop), ev("note x")]).is_none(), "{stop}");
        }
    }

    /// BO-R-014 — the last stop decides, whatever an earlier one was answered by.
    #[test]
    fn ut_waiting_last_stop() {
        let events = [
            ev("stop kind=approval gate=gate2"),
            ev("approved=gate2"),
            ev("stop kind=question"),
        ];
        let w = waiting(&events).unwrap();
        assert_eq!(
            w.args,
            vec![(Some("kind".to_owned()), "question".to_owned())]
        );
    }

    /// BO-R-013 — a valued event's name loses its trailing `=` and keeps its value.
    #[test]
    fn ut_event_entry_valued_name_stripped() {
        let e = event_entry(&ev("approved=gate2"));
        assert_eq!(e.name, "approved");
        assert_eq!(e.value.as_deref(), Some("gate2"));
    }

    /// BO-R-013 — a repeated key keeps its last value at its first position.
    #[test]
    fn ut_event_entry_repeated_key_keeps_last() {
        let e = event_entry(&ev("stop kind=a gate=g kind=b"));
        let args: Vec<_> = e
            .args
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str().unwrap()))
            .collect();
        assert_eq!(args, [("kind", "b"), ("gate", "g")]);
    }

    /// BO-R-013 — positional words are listed in order.
    #[test]
    fn ut_event_entry_positionals_in_order() {
        let e = event_entry(&ev("spec-delta BO-R-011 added origin=impl"));
        assert_eq!(e.positional, ["BO-R-011", "added"]);
    }

    /// BO-R-013 — the free text is kept.
    #[test]
    fn ut_event_entry_text_kept() {
        let e = event_entry(&ev("note some words"));
        assert_eq!(e.text.as_deref(), Some("some words"));
    }

    fn create(s: &mut Store, id: &str, state: board::State, fields: &[(&str, &str)]) {
        let f: Vec<(String, String)> = fields
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        board::create(&mut s.conn, id, state, &f, "t").unwrap();
    }

    fn read(s: &Store, run: Option<&str>, wanted: &[State], archived: bool) -> Dump {
        dump(&Reader::new(&s.conn), run, wanted, archived).unwrap()
    }

    fn seal_at(s: &mut Store, slug: &str, at: &str, abandoned: bool) {
        let dir = tempfile::tempdir().unwrap();
        let layout = crate::project::Layout::new(dir.path().to_path_buf());
        let spec = Seal {
            worktree: None,
            branch: None,
            check: CiCheck::Exempt,
            abandoned,
            checks: vec![],
        };
        seal(&mut s.conn, &layout, slug, at, spec).unwrap();
    }

    fn slugs(d: &Dump) -> Vec<&str> {
        d.runs.iter().map(|r| r.slug.as_str()).collect()
    }

    /// BO-R-018 — the document's version is 1.
    #[test]
    fn ut_dump_version() {
        let s = Store::memory().unwrap();
        let d = read(&s, None, &[], false);
        assert_eq!(d.version, 1);
        assert!(d.runs.is_empty());
    }

    /// BO-R-012 — a run card holds id, state and fields in stored order, and no `stage` key.
    #[test]
    fn ut_dump_run_card_shape() {
        let mut s = Store::memory().unwrap();
        create(
            &mut s,
            "r",
            State::InProgress,
            &[("branch", "b"), ("worktree", "w")],
        );
        let d = read(&s, None, &[], false);
        let card = d.runs[0].card.as_ref().unwrap();
        assert_eq!((card.id.as_str(), card.state.as_str()), ("r", "inprogress"));
        let keys: Vec<_> = card.fields.keys().map(String::as_str).collect();
        assert_eq!(keys, ["branch", "worktree"]);
        let json = serde_json::to_value(card).unwrap();
        assert!(json.get("stage").is_none());
        assert_eq!(json["fields"]["branch"], "b");
    }

    /// BO-R-026 — stage cards come in numeric stage order, each with `s<n>`.
    #[test]
    fn ut_dump_stage_cards_in_stage_order() {
        let mut s = Store::memory().unwrap();
        for id in ["r", "r.s10", "r.s2"] {
            create(&mut s, id, State::Open, &[("branch", "T")]);
        }
        let d = read(&s, None, &[], false);
        let st: Vec<_> = d.runs[0]
            .stages
            .iter()
            .map(|c| (c.id.as_str(), c.stage.as_deref().unwrap(), c.state.as_str()))
            .collect();
        assert_eq!(st, [("r.s2", "s2", "open"), ("r.s10", "s10", "open")]);
        assert_eq!(d.runs[0].stages[0].fields["branch"], "T");
    }

    /// BO-R-027 — live runs come in slug order.
    #[test]
    fn ut_dump_runs_in_slug_order() {
        let mut s = Store::memory().unwrap();
        for id in ["b", "a.s0", "a", "c"] {
            create(&mut s, id, State::Open, &[]);
        }
        assert_eq!(slugs(&read(&s, None, &[], false)), ["a", "b", "c"]);
    }

    /// BO-R-013 — events of the parent and stage cards, oldest first.
    #[test]
    fn ut_dump_events_oldest_first() {
        let mut s = Store::memory().unwrap();
        create(&mut s, "r", State::Open, &[]);
        create(&mut s, "r.s0", State::Open, &[]);
        board::insert_body(&s.conn, "r", "2026-10-04T10:00", "note a");
        board::insert_body(&s.conn, "r.s0", "2026-10-04T10:01", "note b");
        board::insert_body(&s.conn, "r", "2026-10-04T10:02", "note c");
        let d = read(&s, None, &[], false);
        let got: Vec<_> = d.runs[0]
            .events
            .iter()
            .map(|e| (e.card.as_str(), e.at.as_str()))
            .collect();
        assert_eq!(
            got,
            [
                ("r", "2026-10-04T10:00"),
                ("r.s0", "2026-10-04T10:01"),
                ("r", "2026-10-04T10:02")
            ]
        );
    }

    fn shown(s: &Store, run: &str, pos: i64, path: &str, at: &str) {
        s.conn
            .execute(
                "INSERT INTO approvals (run, pos, path, rev, shown_at) VALUES (?1, ?2, ?3, NULL, ?4)",
                rusqlite::params![run, pos, path, at],
            )
            .unwrap();
    }

    /// BO-R-017, BO-R-014 — `waiting` carries the stop's kind, gate, time and the files opened at that time.
    #[test]
    fn ut_dump_waiting_files_of_the_stop() {
        let mut s = Store::memory().unwrap();
        create(&mut s, "r", State::Open, &[]);
        board::insert_body(&s.conn, "r", AT, "stop kind=approval gate=gate2");
        shown(&s, "r", 0, "/x/plan.md", AT);
        shown(&s, "r", 1, "/x/spec.md", AT);
        shown(&s, "r", 2, "/x/old.md", "2026-10-04T09:00");
        let d = read(&s, None, &[], false);
        let w = d.runs[0].waiting.as_ref().unwrap();
        assert_eq!(w.kind.as_deref(), Some("approval"));
        assert_eq!(w.gate.as_deref(), Some("gate2"));
        assert_eq!(w.at, AT);
        assert_eq!(w.files, ["/x/plan.md", "/x/spec.md"]);
    }

    /// BO-E-007 — a stop with no files shown lists an empty `files`.
    #[test]
    fn ut_dump_waiting_without_files() {
        let mut s = Store::memory().unwrap();
        create(&mut s, "r", State::Open, &[]);
        board::insert_body(&s.conn, "r", AT, "stop kind=question");
        shown(&s, "r", 0, "/x/old.md", "2026-10-04T09:00");
        let d = read(&s, None, &[], false);
        assert!(d.runs[0].waiting.as_ref().unwrap().files.is_empty());
    }

    /// BO-E-011 — a later stop in the same minute shows the earlier stop's files.
    #[test]
    fn ut_dump_waiting_files_same_minute() {
        let mut s = Store::memory().unwrap();
        create(&mut s, "r", State::Open, &[]);
        board::insert_body(&s.conn, "r", AT, "stop kind=approval gate=gate2");
        board::insert_body(&s.conn, "r", AT, "stop kind=question");
        shown(&s, "r", 0, "/x/plan.md", AT);
        let d = read(&s, None, &[], false);
        let w = d.runs[0].waiting.as_ref().unwrap();
        assert_eq!(w.kind.as_deref(), Some("question"));
        assert_eq!(w.files, ["/x/plan.md"]);
    }

    /// BO-R-014 — an answered stop leaves `waiting` null.
    #[test]
    fn ut_dump_waiting_answered_is_null() {
        let mut s = Store::memory().unwrap();
        create(&mut s, "r", State::Open, &[]);
        board::insert_body(&s.conn, "r", AT, "stop kind=approval gate=gate2");
        board::insert_body(&s.conn, "r", AT, "approved=gate2");
        assert!(read(&s, None, &[], false).runs[0].waiting.is_none());
    }

    /// BO-R-019 — `--run` limits the document to that slug.
    #[test]
    fn ut_dump_run_filter() {
        let mut s = Store::memory().unwrap();
        for id in ["a", "b", "b.s0"] {
            create(&mut s, id, State::Open, &[]);
        }
        let d = read(&s, Some("b"), &[], false);
        assert_eq!(slugs(&d), ["b"]);
        assert_eq!(d.runs[0].stages.len(), 1);
        assert!(read(&s, Some("none"), &[], false).runs.is_empty());
        seal_at(&mut s, "b", "2026-10-04T12:00", false);
        create(&mut s, "b", State::Open, &[]);
        assert_eq!(slugs(&read(&s, Some("b"), &[], true)), ["b", "b"]);
        assert_eq!(slugs(&read(&s, Some("a"), &[], true)), ["a"]);
    }

    /// BO-R-025 — `--state` keeps runs whose card is in a state, each with all its stage cards.
    #[test]
    fn ut_dump_state_filter_keeps_all_stages() {
        let mut s = Store::memory().unwrap();
        create(&mut s, "a", State::InProgress, &[]);
        create(&mut s, "b", State::InReview, &[]);
        create(&mut s, "b.s0", State::Done, &[]);
        create(&mut s, "b.s1", State::Open, &[]);
        create(&mut s, "c", State::Done, &[]);
        let d = read(&s, None, &[State::InReview], false);
        assert_eq!(slugs(&d), ["b"]);
        assert_eq!(d.runs[0].stages.len(), 2);
        let d = read(&s, None, &[State::InReview, State::Done], false);
        assert_eq!(slugs(&d), ["b", "c"]);
    }

    /// BO-E-013 — stage cards without a run card show `card` null and list those stages.
    #[test]
    fn ut_dump_orphan_stage_cards() {
        let mut s = Store::memory().unwrap();
        create(&mut s, "x.s0", State::Open, &[]);
        create(&mut s, "x.s1", State::Open, &[]);
        let d = read(&s, None, &[], false);
        assert_eq!(slugs(&d), ["x"]);
        assert!(d.runs[0].card.is_none());
        assert_eq!(d.runs[0].stages.len(), 2);
    }

    /// BO-E-014 — `--state` never matches an entry whose `card` is null.
    #[test]
    fn ut_dump_state_skips_entry_without_card() {
        let mut s = Store::memory().unwrap();
        create(&mut s, "x.s0", State::Open, &[]);
        assert_eq!(slugs(&read(&s, None, &[], false)), ["x"]);
        for st in State::ALL {
            assert!(read(&s, None, &[st], false).runs.is_empty());
        }
    }

    /// BO-R-020 — archived runs appear only with the flag.
    #[test]
    fn ut_dump_archived_only_with_flag() {
        let mut s = Store::memory().unwrap();
        create(&mut s, "r", State::Done, &[]);
        seal_at(&mut s, "r", "2026-10-04T12:00", false);
        assert!(read(&s, None, &[], false).runs.is_empty());
        assert_eq!(slugs(&read(&s, None, &[], true)), ["r"]);
    }

    /// BO-R-022 — an archive entry holds the archive's time and abandoned flag.
    #[test]
    fn ut_dump_archived_object() {
        let mut s = Store::memory().unwrap();
        create(&mut s, "r", State::Done, &[("branch", "T")]);
        seal_at(&mut s, "r", "2026-10-04T12:00", true);
        let d = read(&s, None, &[], true);
        let a = d.runs[0].archived.as_ref().unwrap();
        assert_eq!((a.at.as_str(), a.abandoned), ("2026-10-04T12:00", true));
        assert_eq!(d.runs[0].card.as_ref().unwrap().fields["branch"], "T");
        let live = {
            let mut s = Store::memory().unwrap();
            create(&mut s, "r", State::Open, &[]);
            read(&s, None, &[], false)
        };
        assert!(live.runs[0].archived.is_none());
    }

    /// BO-R-024, BO-R-028 — archive entries follow the live ones, oldest first, one per archive.
    #[test]
    fn ut_dump_archive_entries_after_live() {
        let mut s = Store::memory().unwrap();
        create(&mut s, "r", State::Done, &[]);
        seal_at(&mut s, "r", "2026-10-04T12:00", false);
        create(&mut s, "q", State::Done, &[]);
        seal_at(&mut s, "q", "2026-10-04T12:05", false);
        create(&mut s, "r", State::Open, &[]);
        create(&mut s, "a", State::Open, &[]);
        let d = read(&s, None, &[], true);
        assert_eq!(slugs(&d), ["a", "r", "r", "q"]);
        let flags: Vec<_> = d.runs.iter().map(|r| r.archived.is_some()).collect();
        assert_eq!(flags, [false, false, true, true]);
    }

    /// BO-E-008 — an archived run's `waiting` is null; its events are listed oldest first.
    #[test]
    fn ut_dump_archived_waiting_null() {
        let mut s = Store::memory().unwrap();
        create(&mut s, "r", State::Done, &[]);
        create(&mut s, "r.s0", State::Done, &[]);
        board::insert_body(&s.conn, "r.s0", "2026-10-04T10:05", "note b");
        board::insert_body(&s.conn, "r", "2026-10-04T10:01", "note a");
        board::insert_body(&s.conn, "r", "2026-10-04T10:05", "stop kind=question");
        seal_at(&mut s, "r", "2026-10-04T12:00", false);
        let d = read(&s, None, &[], true);
        assert!(d.runs[0].waiting.is_none());
        let got: Vec<_> = d.runs[0]
            .events
            .iter()
            .map(|e| (e.card.as_str(), e.name.as_str()))
            .take(3)
            .collect();
        assert_eq!(got, [("r", "note"), ("r", "stop"), ("r.s0", "note")]);
    }

    /// BO-E-009 — `--state` matches an archived run by its card's state at archiving.
    #[test]
    fn ut_dump_state_matches_archived_card_state() {
        let mut s = Store::memory().unwrap();
        create(&mut s, "r", State::InReview, &[]);
        seal_at(&mut s, "r", "2026-10-04T12:00", false);
        assert_eq!(slugs(&read(&s, None, &[State::InReview], true)), ["r"]);
        assert!(read(&s, None, &[State::Done], true).runs.is_empty());
    }

    /// BO-E-010 — an archive of stage cards only shows `card` null and lists them.
    #[test]
    fn ut_dump_stage_only_archive() {
        let mut s = Store::memory().unwrap();
        create(&mut s, "r.s0", State::Done, &[("branch", "T")]);
        seal_at(&mut s, "r", "2026-10-04T12:00", false);
        let d = read(&s, None, &[], true);
        assert!(d.runs[0].card.is_none());
        assert_eq!(d.runs[0].stages.len(), 1);
        assert_eq!(d.runs[0].stages[0].stage.as_deref(), Some("s0"));
        assert_eq!(d.runs[0].stages[0].fields["branch"], "T");
        assert!(read(&s, None, &[State::Done], true).runs.is_empty());
    }
}
