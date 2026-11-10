//! Test proof: which tests a commit adds, removes or rewrites, and which
//! tests a test run reports failed, per project stack.

pub mod rust;

use std::path::Path;
use std::time::Duration;

use crate::card::board::{LogError, State};
use crate::config::Step;
use crate::run::gauntlet;
use crate::store::Board;

/// The tests a diff adds, removes or rewrites, by name.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TestChanges {
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub rewritten: Vec<String>,
}

/// A stack's way to read tests out of a diff and a test run.
pub trait TestScanner {
    /// The tests a full-context unified diff of `paths()` adds, removes or
    /// rewrites; a test whose body is unchanged in another file is neither.
    fn changes(&self, diff: &str) -> TestChanges;
    /// The tests a test run's output reports failed, as the runner names them.
    fn failed(&self, output: &str) -> Vec<String>;
    /// Program and arguments that run the tests matching `test`.
    fn command(&self, test: &str) -> Vec<String>;
    /// The pathspecs the diff is limited to.
    fn paths(&self) -> &'static [&'static str];
}

/// The Rust scanner for `stack = rust`; `None` on any other stack or none.
#[must_use]
pub fn scanner(stack: Option<&str>) -> Option<Box<dyn TestScanner>> {
    (stack == Some("rust")).then(|| Box::new(rust::RustScanner) as Box<dyn TestScanner>)
}

/// Why `red` or `pins` records nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProofRefused {
    Card,
    State(State),
    /// Not a Rust project.
    Stack,
    /// The run reported no failed test.
    NoFailure,
    /// The test command could not be started or timed out.
    Run,
}

/// The states a card takes proofs in.
fn takes_proofs(state: State) -> bool {
    matches!(state, State::InProgress | State::InReview)
}

/// The card's state check shared by `red` and `pins`.
fn gate(db: &Board, card: &str) -> rusqlite::Result<Result<(), ProofRefused>> {
    Ok(match db.read(|r| r.state(card))? {
        None => Err(ProofRefused::Card),
        Some(s) if !takes_proofs(s) => Err(ProofRefused::State(s)),
        Some(_) => Ok(()),
    })
}

/// Runs the tests matching `test` in `wt`; logs one `test-red` per failed test.
///
/// # Errors
///
/// The state database failed.
pub fn red(
    db: &mut Board,
    wt: &Path,
    card: &str,
    stack: Option<&str>,
    test: &str,
    mutation: Option<&str>,
    now: &str,
) -> rusqlite::Result<Result<Vec<String>, ProofRefused>> {
    if let Err(refused) = gate(db, card)? {
        return Ok(Err(refused));
    }
    let Some(scanner) = scanner(stack) else {
        return Ok(Err(ProofRefused::Stack));
    };
    red_with(db, wt, card, scanner.as_ref(), test, mutation, now)
}

/// How long a `stage red` run may take.
const RED_TIMEOUT: Duration = Duration::from_mins(30);

/// The output of `scanner`'s command for `test` in `wt`, `None` when it
/// could not start or timed out.
fn run_tests(wt: &Path, scanner: &dyn TestScanner, test: &str) -> Option<String> {
    let log = std::env::temp_dir().join(format!(
        "ferrule-red-{}-{}.log",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos())
    ));
    let step = Step {
        name: "red".to_owned(),
        cmd: scanner.command(test),
        timeout: RED_TIMEOUT,
        coverage: None,
    };
    let result = gauntlet::run_step(wt, &step, &log);
    let output = std::fs::read_to_string(&log).ok();
    let _ = std::fs::remove_file(&log);
    match result {
        // A failing test run is the point; only an unstartable or timed-out one is not.
        Ok(gauntlet::StepResult::Ok | gauntlet::StepResult::Failed) => {
            let out = output?;
            (!out.contains("ferrule: could not start ")).then_some(out)
        }
        _ => None,
    }
}

fn red_with(
    db: &mut Board,
    wt: &Path,
    card: &str,
    scanner: &dyn TestScanner,
    test: &str,
    mutation: Option<&str>,
    now: &str,
) -> rusqlite::Result<Result<Vec<String>, ProofRefused>> {
    if let Err(refused) = gate(db, card)? {
        return Ok(Err(refused));
    }
    let Some(output) = run_tests(wt, scanner, test) else {
        return Ok(Err(ProofRefused::Run));
    };
    let failed = scanner.failed(&output);
    if failed.is_empty() {
        return Ok(Err(ProofRefused::NoFailure));
    }
    db.write(|w| {
        for name in &failed {
            let mut args = vec![format!("test={name}")];
            if let Some(m) = mutation {
                args.push(format!("mutation={m}"));
            }
            w.log(card, "test-red", &args, now).map_err(log_failed)?;
        }
        Ok::<_, rusqlite::Error>(())
    })?;
    Ok(Ok(failed))
}

/// A line the vocabulary refuses is a bug of the caller, not a refusal.
fn log_failed(e: LogError) -> rusqlite::Error {
    match e {
        LogError::Sql(e) => e,
        e => rusqlite::Error::ToSqlConversionFailure(e.to_string().into()),
    }
}

/// Logs `test-pins test=<test> -- <why>`.
///
/// # Errors
///
/// The state database failed.
pub fn pins(
    db: &mut Board,
    card: &str,
    stack: Option<&str>,
    test: &str,
    why: &str,
    now: &str,
) -> rusqlite::Result<Result<(), ProofRefused>> {
    if let Err(refused) = gate(db, card)? {
        return Ok(Err(refused));
    }
    if scanner(stack).is_none() {
        return Ok(Err(ProofRefused::Stack));
    }
    let args = vec![format!("test={test}"), "--".to_owned(), why.to_owned()];
    db.write(|w| w.log(card, "test-pins", &args, now).map_err(log_failed))?;
    Ok(Ok(()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::card::board::State;

    const NOW: &str = "2026-11-10T12:00";

    /// Reports `a::t_red` failed through a shell, whatever the test asked.
    struct Fake(&'static str);

    impl TestScanner for Fake {
        fn changes(&self, _diff: &str) -> TestChanges {
            TestChanges::default()
        }
        fn failed(&self, output: &str) -> Vec<String> {
            output
                .lines()
                .filter_map(|l| l.strip_prefix("test ")?.strip_suffix(" ... FAILED"))
                .map(str::to_owned)
                .collect()
        }
        fn command(&self, _test: &str) -> Vec<String> {
            ["sh", "-c", self.0].map(str::to_owned).to_vec()
        }
        fn paths(&self) -> &'static [&'static str] {
            &[]
        }
    }

    const RED: &str = "printf 'test a::t_red ... FAILED\\ntest a::t_ok ... ok\\n'";

    fn board(state: State) -> Board {
        let mut db = Board::memory().unwrap();
        db.write(|w| w.create("demo", State::Open, &[], NOW))
            .unwrap();
        db.write(|w| w.create("demo.s1", state, &[], NOW)).unwrap();
        db
    }

    fn lines(db: &Board, event: &str) -> Vec<String> {
        db.read(|r| r.log_lines("demo.s1", Some(event)))
            .unwrap()
            .into_iter()
            .map(|(_, body)| body)
            .collect()
    }

    /// GW-R-068 — each test the run reports failed gets one `test-red` line, a passing test none.
    #[test]
    fn ut_red_logs_one_line_per_failure() {
        let mut db = board(State::InProgress);
        let got = red_with(
            &mut db,
            Path::new("."),
            "demo.s1",
            &Fake(RED),
            "t_",
            None,
            NOW,
        )
        .unwrap();
        assert_eq!(got, Ok(vec!["a::t_red".to_owned()]));
        assert_eq!(lines(&db, "test-red"), ["test-red test=a::t_red"]);
    }

    /// GW-E-019 — a run that reports no failed test logs nothing.
    #[test]
    fn ut_red_with_no_failure_logs_nothing() {
        let mut db = board(State::InProgress);
        let got = red_with(
            &mut db,
            Path::new("."),
            "demo.s1",
            &Fake("printf 'test a::t_ok ... ok\\n'"),
            "t_",
            None,
            NOW,
        )
        .unwrap();
        assert_eq!(got, Err(ProofRefused::NoFailure));
        assert!(lines(&db, "test-red").is_empty());
    }

    #[test]
    fn ut_red_and_pins_refuse_off_rust() {
        let mut db = board(State::InProgress);
        let got = red(&mut db, Path::new("."), "demo.s1", None, "t", None, NOW).unwrap();
        assert_eq!(got, Err(ProofRefused::Stack));
        let got = pins(&mut db, "demo.s1", Some("node"), "t", "why", NOW).unwrap();
        assert_eq!(got, Err(ProofRefused::Stack));
        assert!(lines(&db, "test-pins").is_empty());
    }

    #[test]
    fn ut_red_refuses_a_missing_card_and_a_state() {
        let mut db = board(State::Open);
        let got = red_with(
            &mut db,
            Path::new("."),
            "demo.s9",
            &Fake(RED),
            "t",
            None,
            NOW,
        )
        .unwrap();
        assert_eq!(got, Err(ProofRefused::Card));
        let got = red_with(
            &mut db,
            Path::new("."),
            "demo.s1",
            &Fake(RED),
            "t",
            None,
            NOW,
        )
        .unwrap();
        assert_eq!(got, Err(ProofRefused::State(State::Open)));
        let got = pins(&mut db, "demo.s9", Some("rust"), "t", "why", NOW).unwrap();
        assert_eq!(got, Err(ProofRefused::Card));
        assert!(lines(&db, "test-red").is_empty());
    }

    #[test]
    fn ut_scanner_is_rust_only() {
        assert!(scanner(Some("rust")).is_some());
        assert!(scanner(Some("node")).is_none());
        assert!(scanner(None).is_none());
    }
}
