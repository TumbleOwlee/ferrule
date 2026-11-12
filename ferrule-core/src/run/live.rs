//! The run's live check: while it is open, `ferrule hook shell` lets a
//! `gh api` write through to the repository the run named at gate 2.

use crate::card::board::{self, LogError};
use crate::card::typed::{Name, Typed};
use crate::store::{Board, Reader};

/// Why a live check was not opened or closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiveRefused {
    /// No such card, or the board refused the line.
    Card,
    /// RU-E-019: the run names no `live-repo`.
    NoRepo,
    /// RU-E-038: a code stage of the run has no logged commit.
    OpenStage { stage: u32 },
}

fn log(
    db: &mut Board,
    slug: &str,
    line: &str,
    now: &str,
) -> rusqlite::Result<Result<(), LiveRefused>> {
    match db.write(|w| w.log(slug, line, &[], now)) {
        Ok(()) => Ok(Ok(())),
        Err(LogError::Sql(e)) => Err(e),
        Err(_) => Ok(Err(LiveRefused::Card)),
    }
}

/// Logs `live-check=open`; the named repository.
///
/// # Errors
/// The SQL error of the board.
pub fn start(
    db: &mut Board,
    slug: &str,
    now: &str,
) -> rusqlite::Result<Result<String, LiveRefused>> {
    let repo = db.read(|r| {
        if r.state(slug)?.is_none() {
            return Ok::<_, rusqlite::Error>(Err(LiveRefused::Card));
        }
        for (n, id, _) in board::stages(r.conn(), slug)? {
            if n != 0 && board::last_value(r.conn(), &id, Name::Commit)?.is_none() {
                return Ok(Err(LiveRefused::OpenStage { stage: n }));
            }
        }
        Ok(match r.field(slug, "live-repo")? {
            Some(repo) if !repo.is_empty() => Ok(repo),
            _ => Err(LiveRefused::NoRepo),
        })
    })?;
    let Ok(repo) = repo else {
        return Ok(repo);
    };
    Ok(log(db, slug, "live-check=open", now)?.map(|()| repo))
}

/// Logs `live-check=closed`.
///
/// # Errors
/// The SQL error of the board.
pub fn end(db: &mut Board, slug: &str, now: &str) -> rusqlite::Result<Result<(), LiveRefused>> {
    if db.read(|r| r.state(slug))?.is_none() {
        return Ok(Err(LiveRefused::Card));
    }
    log(db, slug, "live-check=closed", now)
}

/// `live-repo` of every run whose last `live-check=` is `open`; none when
/// the board cannot be read.
#[must_use]
pub fn open_repos(r: &Reader<'_>) -> Vec<String> {
    let read = || -> rusqlite::Result<Vec<String>> {
        let mut out = Vec::new();
        for (id, _) in r.all()? {
            let last = r.events(&id, &[Name::LiveCheck])?;
            if !matches!(
                last.last().map(|e| e.typed()),
                Some(Typed::LiveCheck { open: true })
            ) {
                continue;
            }
            if let Some(repo) = r.field(&id, "live-repo")?.filter(|v| !v.is_empty()) {
                out.push(repo);
            }
        }
        Ok(out)
    };
    read().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::card::board::State;

    const NOW: &str = "2026-11-10T12:00";

    fn board(repo: Option<&str>) -> Board {
        let mut db = Board::memory().unwrap();
        let fields: Vec<(String, String)> = repo
            .map(|r| ("live-repo".to_owned(), r.to_owned()))
            .into_iter()
            .collect();
        db.write(|w| w.create("demo", State::Open, &fields, NOW))
            .unwrap();
        db
    }

    fn open(db: &Board) -> Vec<String> {
        db.read(|r| Ok::<_, rusqlite::Error>(open_repos(r)))
            .unwrap()
    }

    /// RU-E-019 — a run that names no repository cannot open a live check; a missing card is refused.
    #[test]
    fn ut_start_needs_a_named_repository() {
        let mut db = board(None);
        assert_eq!(
            start(&mut db, "demo", NOW).unwrap(),
            Err(LiveRefused::NoRepo)
        );
        assert_eq!(start(&mut db, "nope", NOW).unwrap(), Err(LiveRefused::Card));
        assert_eq!(end(&mut db, "nope", NOW).unwrap(), Err(LiveRefused::Card));
        assert!(open(&db).is_empty());
    }

    /// RU-E-038 — `start` refuses while a code stage has no logged commit, before the repository check.
    #[test]
    fn ut_start_refuses_an_uncommitted_stage() {
        let mut db = board(Some("o/r"));
        for n in 0..=2 {
            db.write(|w| w.create(&format!("demo.s{n}"), State::Open, &[], NOW))
                .unwrap();
        }
        db.write(|w| w.log("demo.s1", "commit=abc1234", &[], NOW))
            .unwrap();
        assert_eq!(
            start(&mut db, "demo", NOW).unwrap(),
            Err(LiveRefused::OpenStage { stage: 2 })
        );
        db.write(|w| w.log("demo.s2", "commit=abc1235", &[], NOW))
            .unwrap();
        assert_eq!(start(&mut db, "demo", NOW).unwrap(), Ok("o/r".to_owned()));

        let mut db = board(None);
        db.write(|w| w.create("demo.s1", State::Open, &[], NOW))
            .unwrap();
        assert_eq!(
            start(&mut db, "demo", NOW).unwrap(),
            Err(LiveRefused::OpenStage { stage: 1 })
        );
    }

    /// RU-R-043, RU-R-044 — `start` opens the check and names the repository, `end` closes it.
    #[test]
    fn ut_start_opens_and_end_closes() {
        let mut db = board(Some("o/r"));
        assert!(open(&db).is_empty());
        assert_eq!(start(&mut db, "demo", NOW).unwrap(), Ok("o/r".to_owned()));
        assert_eq!(open(&db), ["o/r"]);
        assert_eq!(end(&mut db, "demo", NOW).unwrap(), Ok(()));
        assert!(open(&db).is_empty());
    }

    /// SG-E-012 — an open check whose `live-repo` was cleared opens nothing.
    #[test]
    fn ut_a_cleared_repository_opens_nothing() {
        let mut db = board(Some("o/r"));
        start(&mut db, "demo", NOW).unwrap().unwrap();
        db.write(|w| w.set_all("demo", &[("live-repo".to_owned(), String::new())]))
            .unwrap();
        assert!(open(&db).is_empty());
    }
}
