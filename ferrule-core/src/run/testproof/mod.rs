//! Test proof: which tests a commit adds, removes or rewrites, and which
//! tests a test run reports failed, per configured test command.

mod detect;
#[allow(dead_code, reason = "read by the detector")]
mod syntax;

pub use detect::Detector;

use std::path::Path;
use std::time::Duration;

use crate::card::board::{LogError, State};
use crate::card::typed::{Name, Typed};
use crate::config::Step;
use crate::run::commit::CommitRefused;
use crate::run::gauntlet;
use crate::store::{Board, Reader};

/// The tests a diff adds, removes or rewrites, by name.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TestChanges {
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub rewritten: Vec<String>,
}

/// What the commit checks need: the configured test commands and the
/// replacements named.
#[derive(Debug, Clone, Copy)]
pub struct TestCheck<'a> {
    pub commands: &'a TestCommands,
    /// `--replaced <old>=<new>`.
    pub replaced: &'a [(String, String)],
}

/// Why a stage commit applied a test check to no test of some command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Skipped {
    /// No test command is configured.
    NoCommand,
    /// Commands declaring no test files and pattern; their prefixes, empty
    /// when unprefixed.
    NoDetection(Vec<String>),
}

/// Refuses unproven new tests and unreplaced removals or rewrites; proofs
/// are read from `card` only.
///
/// # Errors
///
/// The state database failed.
pub fn check(
    r: &Reader<'_>,
    card: &str,
    changes: &TestChanges,
    replaced: &[(String, String)],
) -> rusqlite::Result<Result<(), CommitRefused>> {
    let events = r.events(card, &[Name::TestRed, Name::TestPins])?;
    let proven: Vec<&str> = events
        .iter()
        .filter_map(|e| match e.typed() {
            Typed::TestRed { test: Some(t), .. } | Typed::TestPins { test: Some(t) } => Some(t),
            _ => None,
        })
        .collect();
    let own = |t: &String| replaced.iter().any(|(old, new)| old == t && new == t);
    let unproven: Vec<String> = changes
        .added
        .iter()
        .chain(changes.rewritten.iter().filter(|t| own(t)))
        .filter(|t| !proven.contains(&t.as_str()))
        .cloned()
        .collect();
    if !unproven.is_empty() {
        return Ok(Err(CommitRefused::TestUnproven { tests: unproven }));
    }
    let unreplaced: Vec<String> = changes
        .removed
        .iter()
        .chain(&changes.rewritten)
        .filter(|t| !replaced.iter().any(|(old, _)| old == *t))
        .cloned()
        .collect();
    if !unreplaced.is_empty() {
        return Ok(Err(CommitRefused::TestUnreplaced { tests: unreplaced }));
    }
    Ok(Ok(()))
}

/// Why `red` or `pins` records nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProofRefused {
    Card,
    State(State),
    /// No test command is configured.
    NoCommand,
    /// The name starts with none of the configured prefixes.
    Prefix(Vec<String>),
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

/// Runs the tests matching `test` in `wt` with the test command its prefix
/// picks; logs one `test-red` for the named test when the run failed it.
///
/// # Errors
///
/// The state database failed.
pub fn red(
    db: &mut Board,
    wt: &Path,
    card: &str,
    commands: &TestCommands,
    test: &str,
    mutation: Option<&str>,
    now: &str,
) -> rusqlite::Result<Result<Vec<String>, ProofRefused>> {
    if let Err(refused) = gate(db, card)? {
        return Ok(Err(refused));
    }
    let (command, bare) = match commands.pick(test) {
        Pick::Found { command, bare } => (command, bare),
        Pick::NoCommand => return Ok(Err(ProofRefused::NoCommand)),
        Pick::Prefix(p) => return Ok(Err(ProofRefused::Prefix(p))),
    };
    let Some((result, output)) = run_tests(wt, command, bare) else {
        return Ok(Err(ProofRefused::Run));
    };
    if !command.judge(bare, result, &output) {
        return Ok(Err(ProofRefused::NoFailure));
    }
    db.write(|w| {
        let mut args = vec![format!("test={test}")];
        if let Some(m) = mutation {
            args.push(format!("mutation={m}"));
        }
        w.log(card, "test-red", &args, now).map_err(log_failed)
    })?;
    Ok(Ok(vec![test.to_owned()]))
}

/// The result and output of `command` for `test` in `wt`, `None` when it
/// could not start or timed out.
fn run_tests(
    wt: &Path,
    command: &TestCommand,
    test: &str,
) -> Option<(gauntlet::StepResult, String)> {
    let log = std::env::temp_dir().join(format!(
        "ferrule-red-{}-{}.log",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos())
    ));
    let step = Step {
        name: "red".to_owned(),
        cmd: command.argv(test),
        timeout: command.timeout,
        coverage: None,
        at: vec![crate::config::Level::Full],
    };
    let result = gauntlet::run_step(wt, &step, &log);
    let output = std::fs::read_to_string(&log).ok();
    let _ = std::fs::remove_file(&log);
    match result {
        // A failing test run is the point; only an unstartable or timed-out one is not.
        Ok(r @ (gauntlet::StepResult::Ok | gauntlet::StepResult::Failed)) => {
            let out = output?;
            // The step runner writes this line right after its header line.
            let spawn_failed = out
                .lines()
                .nth(1)
                .is_some_and(|l| l.starts_with("ferrule: could not start "));
            (!spawn_failed).then_some((r, out))
        }
        _ => None,
    }
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
    commands: &TestCommands,
    test: &str,
    why: &str,
    now: &str,
) -> rusqlite::Result<Result<(), ProofRefused>> {
    if let Err(refused) = gate(db, card)? {
        return Ok(Err(refused));
    }
    match commands.pick(test) {
        Pick::Found { .. } => {}
        Pick::NoCommand => return Ok(Err(ProofRefused::NoCommand)),
        Pick::Prefix(p) => return Ok(Err(ProofRefused::Prefix(p))),
    }
    let args = vec![format!("test={test}"), "--".to_owned(), why.to_owned()];
    db.write(|w| w.log(card, "test-pins", &args, now).map_err(log_failed))?;
    Ok(Ok(()))
}

/// The command one test runs with: a template whose `{test}` stands for the
/// test's name, how a failure shows in its output, how its tests are found
/// in a diff, and the time it may take.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestCommand {
    prefix: Option<String>,
    template: Vec<String>,
    /// Source of the failure pattern, `{test}` still in it.
    failed: Option<String>,
    pub detect: Option<Detector>,
    pub timeout: Duration,
}

impl TestCommand {
    /// The prefix a test name carries for this command.
    #[must_use]
    pub fn prefix(&self) -> Option<&str> {
        self.prefix.as_deref()
    }

    /// The template with every `{test}` replaced by `bare`.
    #[must_use]
    pub fn argv(&self, bare: &str) -> Vec<String> {
        self.template
            .iter()
            .map(|a| a.replace("{test}", bare))
            .collect()
    }

    /// Whether the run failed the test `bare`: with a failure pattern, any
    /// output line matches it; without, the command exited non-zero.
    #[must_use]
    pub(crate) fn judge(&self, bare: &str, result: gauntlet::StepResult, output: &str) -> bool {
        match &self.failed {
            Some(source) => regex::Regex::new(&source.replace("{test}", &regex::escape(bare)))
                .is_ok_and(|re| output.lines().any(|l| re.is_match(l))),
            None => matches!(result, gauntlet::StepResult::Failed),
        }
    }
}

/// The configured test commands, checked.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TestCommands {
    commands: Vec<TestCommand>,
}

/// The command a test name picks.
#[derive(Debug)]
pub enum Pick<'a> {
    Found {
        command: &'a TestCommand,
        /// The name without the command's prefix.
        bare: &'a str,
    },
    NoCommand,
    /// The name starts with none of these prefixes.
    Prefix(Vec<String>),
}

impl TestCommands {
    /// Compiles and checks the configured commands.
    ///
    /// # Errors
    ///
    /// A command without `{test}` in `run`, one of several without a prefix,
    /// a prefix starting another, a pattern that is no regular expression, a
    /// declaration pattern without a `name` group, or `files` and `declare`
    /// not set together; the message names the command.
    pub fn from_config(test: &crate::config::Test) -> Result<TestCommands, String> {
        let several = test.command.len() > 1;
        let mut commands = Vec::new();
        for (n, c) in test.command.iter().enumerate() {
            let who = c.prefix.as_ref().map_or_else(
                || format!("test command {}", n + 1),
                |p| format!("test command `{p}`"),
            );
            if !c.run.iter().any(|a| a.contains("{test}")) {
                return Err("test command needs a {test} argument".to_owned());
            }
            if several && c.prefix.as_deref().is_none_or(str::is_empty) {
                return Err(format!(
                    "{who} needs a prefix: several test commands are configured"
                ));
            }
            let check = |source: &str| regex::Regex::new(&source.replace("{test}", "x"));
            if let Some(f) = &c.failed {
                check(f).map_err(|e| format!("{who}: failed is no regular expression: {e}"))?;
            }
            if c.files.is_empty() != c.declare.is_none() {
                return Err(format!("{who}: files and declare go together"));
            }
            let detect = match &c.declare {
                Some(d) => {
                    let declare = regex::Regex::new(d)
                        .map_err(|e| format!("{who}: declare is no regular expression: {e}"))?;
                    if !declare.capture_names().flatten().any(|g| g == "name") {
                        return Err(format!("{who}: declare needs a group named `name`"));
                    }
                    Some(Detector {
                        files: c.files.clone(),
                        declare,
                    })
                }
                None => None,
            };
            commands.push(TestCommand {
                prefix: c.prefix.clone().filter(|p| !p.is_empty()),
                template: c.run.clone(),
                failed: c.failed.clone(),
                detect,
                timeout: test.timeout,
            });
        }
        for a in &commands {
            for b in &commands {
                if let (Some(pa), Some(pb)) = (&a.prefix, &b.prefix)
                    && !std::ptr::eq(a, b)
                    && pb.starts_with(pa.as_str())
                {
                    return Err(format!("test prefix `{pa}` is the start of `{pb}`"));
                }
            }
        }
        Ok(TestCommands { commands })
    }

    /// The command whose prefix `name` starts with, and the name without it;
    /// a lone unprefixed command takes the name as given.
    #[must_use]
    pub fn pick<'a>(&'a self, name: &'a str) -> Pick<'a> {
        if self.commands.is_empty() {
            return Pick::NoCommand;
        }
        for command in &self.commands {
            if let Some(p) = command.prefix()
                && let Some(bare) = name.strip_prefix(p)
            {
                return Pick::Found { command, bare };
            }
        }
        match self.commands.as_slice() {
            [command] if command.prefix.is_none() => Pick::Found {
                command,
                bare: name,
            },
            _ => Pick::Prefix(self.prefixes()),
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = &TestCommand> {
        self.commands.iter()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }

    /// The configured prefixes, in order.
    #[must_use]
    pub fn prefixes(&self) -> Vec<String> {
        self.commands
            .iter()
            .filter_map(|c| c.prefix.clone())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::card::board::State;
    use crate::config::{Test, TestCommandConfig};

    const NOW: &str = "2026-11-10T12:00";

    fn strings(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| (*s).to_owned()).collect()
    }

    fn cfg(prefix: Option<&str>, run: &[&str], failed: Option<&str>) -> TestCommandConfig {
        TestCommandConfig {
            prefix: prefix.map(str::to_owned),
            run: strings(run),
            failed: failed.map(str::to_owned),
            files: Vec::new(),
            declare: None,
        }
    }

    fn cmds(list: Vec<TestCommandConfig>) -> TestCommands {
        let t = Test {
            command: list,
            timeout: Duration::from_secs(30),
        };
        TestCommands::from_config(&t).unwrap()
    }

    /// One command that runs the shell script, with `{test}` available to it.
    fn sh(prefix: Option<&str>, script: &str, failed: Option<&str>) -> TestCommands {
        cmds(vec![cfg(prefix, &["sh", "-c", script], failed)])
    }

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

    fn red_in(
        db: &mut Board,
        commands: &TestCommands,
        name: &str,
    ) -> Result<Vec<String>, ProofRefused> {
        red(db, Path::new("."), "demo.s1", commands, name, None, NOW).unwrap()
    }

    /// GW-R-096 — the prefix picks the command and is stripped before substitution.
    #[test]
    fn ut_pick_strips_the_matching_prefix() {
        let c = cmds(vec![
            cfg(Some("cargo:"), &["cargo", "{test}"], None),
            cfg(Some("vitest:"), &["vt", "{test}"], None),
        ]);
        let Pick::Found { command, bare } = c.pick("vitest:adds two") else {
            panic!("no command");
        };
        assert_eq!(bare, "adds two");
        assert_eq!(command.argv(bare), strings(&["vt", "adds two"]));
    }

    /// GW-R-104 — a lone unprefixed command takes the name as given.
    #[test]
    fn ut_pick_lone_command_takes_the_name_as_given() {
        let c = cmds(vec![cfg(None, &["go", "{test}"], None)]);
        assert!(matches!(c.pick("a::b"), Pick::Found { bare: "a::b", .. }));
    }

    /// GW-R-106 — a name with no configured prefix is refused, naming the prefixes.
    #[test]
    fn ut_pick_unmatched_name_names_the_prefixes() {
        let c = cmds(vec![
            cfg(Some("a:"), &["x", "{test}"], None),
            cfg(Some("b:"), &["y", "{test}"], None),
        ]);
        let Pick::Prefix(p) = c.pick("t") else {
            panic!("picked");
        };
        assert_eq!(p, strings(&["a:", "b:"]));
    }

    /// GW-R-106 — a lone prefixed command refuses a bare name.
    #[test]
    fn ut_pick_lone_prefixed_command_refuses_a_bare_name() {
        let c = cmds(vec![cfg(Some("a:"), &["x", "{test}"], None)]);
        assert!(matches!(c.pick("t"), Pick::Prefix(p) if p == ["a:"]));
        assert!(matches!(c.pick("a:t"), Pick::Found { bare: "t", .. }));
    }

    /// GW-E-029 — no configured command picks nothing, for `red` and `pins` alike.
    #[test]
    fn ut_pick_no_command() {
        let c = cmds(Vec::new());
        assert!(matches!(c.pick("t"), Pick::NoCommand));
        let mut db = board(State::InProgress);
        assert_eq!(red_in(&mut db, &c, "t"), Err(ProofRefused::NoCommand));
        let got = pins(&mut db, "demo.s1", &c, "t", "why", NOW).unwrap();
        assert_eq!(got, Err(ProofRefused::NoCommand));
        assert!(lines(&db, "test-pins").is_empty());
    }

    /// GW-R-110 — `pins` refuses an unprefixed name while prefixes exist and records nothing.
    #[test]
    fn ut_pins_refuses_an_unprefixed_name_while_prefixes_exist() {
        let c = cmds(vec![cfg(Some("a:"), &["x", "{test}"], None)]);
        let mut db = board(State::InProgress);
        let got = pins(&mut db, "demo.s1", &c, "t", "why", NOW).unwrap();
        assert_eq!(got, Err(ProofRefused::Prefix(strings(&["a:"]))));
        assert!(lines(&db, "test-pins").is_empty());
        let got = pins(&mut db, "demo.s1", &c, "a:t", "why", NOW).unwrap();
        assert_eq!(got, Ok(()));
        assert_eq!(lines(&db, "test-pins"), ["test-pins test=a:t why"]);
    }

    fn judged(command: &TestCommands, bare: &str, result: gauntlet::StepResult, out: &str) -> bool {
        let Pick::Found { command, .. } = command.pick(bare) else {
            panic!("no command");
        };
        command.judge(bare, result, out)
    }

    /// GW-R-107 — a failure pattern matches the output line by line.
    #[test]
    fn ut_judge_matches_the_pattern_line_by_line() {
        let c = sh(None, "true {test}", Some("^FAIL {test}$"));
        let ok = || gauntlet::StepResult::Ok;
        assert!(judged(&c, "t_x", ok(), "a\nFAIL t_x\nb"));
        assert!(!judged(&c, "t_x", ok(), "FAIL t_x extra\nFAIL t_y"));
    }

    /// GW-R-107 — the name is escaped inside the failure pattern.
    #[test]
    fn ut_judge_escapes_the_name() {
        let c = sh(None, "true {test}", Some("^FAIL {test}$"));
        let ok = || gauntlet::StepResult::Ok;
        assert!(judged(&c, "a.b", ok(), "FAIL a.b"));
        assert!(!judged(&c, "a.b", ok(), "FAIL aXb"));
    }

    /// GW-E-039 — with a failure pattern the exit code does not decide.
    #[test]
    fn ut_judge_pattern_ignores_exit_code() {
        let c = sh(None, "true {test}", Some("^FAIL {test}$"));
        assert!(!judged(&c, "t", gauntlet::StepResult::Failed, "boom"));
        assert!(judged(&c, "t", gauntlet::StepResult::Ok, "FAIL t"));
    }

    /// GW-R-098, GW-E-034, GW-E-030 — without a pattern the exit decides, whatever the output.
    #[test]
    fn ut_judge_without_pattern_reads_the_exit() {
        let c = sh(None, "true {test}", None);
        assert!(judged(&c, "t", gauntlet::StepResult::Failed, ""));
        assert!(!judged(&c, "t", gauntlet::StepResult::Ok, "FAIL t"));
    }

    /// GW-R-073, GW-R-096 — `red` runs the picked command with the bare name and logs the given name.
    #[test]
    fn ut_red_logs_the_named_test_with_its_prefix() {
        let c = cmds(vec![
            cfg(Some("a:"), &["sh", "-c", "exit 0 # {test}"], None),
            cfg(
                Some("b:"),
                &["sh", "-c", "echo 'not ok {test}'"],
                Some("^not ok {test}$"),
            ),
        ]);
        let mut db = board(State::InProgress);
        let got = red_in(&mut db, &c, "b:t_x");
        assert_eq!(got, Ok(vec!["b:t_x".to_owned()]));
        assert_eq!(lines(&db, "test-red"), ["test-red test=b:t_x"]);
        assert_eq!(red_in(&mut db, &c, "a:t_x"), Err(ProofRefused::NoFailure));
        assert_eq!(lines(&db, "test-red").len(), 1);
    }

    /// GW-R-073 — a mutation proof names the mutation on the one failure.
    #[test]
    fn ut_red_logs_a_mutation() {
        let c = sh(None, "exit 1 # {test}", None);
        let mut db = board(State::InProgress);
        let got = red(
            &mut db,
            Path::new("."),
            "demo.s1",
            &c,
            "t_x",
            Some("broke it"),
            NOW,
        )
        .unwrap();
        assert_eq!(got, Ok(vec!["t_x".to_owned()]));
        assert_eq!(
            lines(&db, "test-red"),
            ["test-red test=t_x mutation=\"broke it\""]
        );
    }

    /// GW-R-099 — a run past the command's timeout is refused with `Run`.
    #[test]
    fn ut_red_refuses_a_run_past_its_timeout() {
        let t = Test {
            command: vec![cfg(None, &["sh", "-c", "sleep 5 # {test}"], None)],
            timeout: Duration::from_secs(1),
        };
        let c = TestCommands::from_config(&t).unwrap();
        let mut db = board(State::InProgress);
        assert_eq!(red_in(&mut db, &c, "t"), Err(ProofRefused::Run));
        assert!(lines(&db, "test-red").is_empty());
    }

    /// A command that cannot start is `Run`, never a recorded failure.
    #[test]
    fn ut_red_refuses_an_unstartable_command() {
        let c = cmds(vec![cfg(None, &["ferrule-no-such-program-{test}"], None)]);
        let mut db = board(State::InProgress);
        assert_eq!(red_in(&mut db, &c, "t"), Err(ProofRefused::Run));
    }

    #[test]
    fn ut_red_refuses_a_missing_card_and_a_state() {
        let c = sh(None, "exit 1 # {test}", None);
        let mut db = board(State::Open);
        let got = red(&mut db, Path::new("."), "demo.s9", &c, "t", None, NOW).unwrap();
        assert_eq!(got, Err(ProofRefused::Card));
        assert_eq!(
            red_in(&mut db, &c, "t"),
            Err(ProofRefused::State(State::Open))
        );
        let got = pins(&mut db, "demo.s9", &c, "t", "why", NOW).unwrap();
        assert_eq!(got, Err(ProofRefused::Card));
        assert!(lines(&db, "test-red").is_empty());
    }

    fn log(db: &mut Board, card: &str, event: &str, args: &[&str]) {
        let args: Vec<String> = args.iter().map(|a| (*a).to_owned()).collect();
        db.write(|w| w.log(card, event, &args, NOW)).unwrap();
    }

    fn checked(
        db: &Board,
        card: &str,
        changes: &TestChanges,
        replaced: &[(String, String)],
    ) -> Result<(), CommitRefused> {
        db.read(|r| check(r, card, changes, replaced)).unwrap()
    }

    fn added(names: &[&str]) -> TestChanges {
        TestChanges {
            added: strings(names),
            ..TestChanges::default()
        }
    }

    /// GW-R-075 — a `test-red` on another card leaves this card's test unproven.
    #[test]
    fn ut_check_reads_proofs_of_its_own_card_only() {
        let mut db = board(State::InReview);
        db.write(|w| w.create("demo.s2", State::InProgress, &[], NOW))
            .unwrap();
        log(&mut db, "demo.s2", "test-red", &["test=t_new"]);
        let got = checked(&db, "demo.s1", &added(&["t_new"]), &[]);
        assert_eq!(
            got,
            Err(CommitRefused::TestUnproven {
                tests: vec!["t_new".to_owned()]
            })
        );
        assert_eq!(checked(&db, "demo.s2", &added(&["t_new"]), &[]), Ok(()));
    }

    /// GW-R-069 — a proof counts only under the exact prefixed name.
    #[test]
    fn ut_check_matches_exact_prefixed_names() {
        let mut db = board(State::InReview);
        log(&mut db, "demo.s1", "test-red", &["test=a:t_one"]);
        log(
            &mut db,
            "demo.s1",
            "test-pins",
            &["test=a:t_two", "--", "why"],
        );
        let got = checked(
            &db,
            "demo.s1",
            &added(&["a:t_one", "a:t_two", "t_one", "b:t_one"]),
            &[],
        );
        assert_eq!(
            got,
            Err(CommitRefused::TestUnproven {
                tests: strings(&["t_one", "b:t_one"])
            })
        );
    }

    /// A `test-red` line without `test=` proves nothing.
    #[test]
    fn ut_check_ignores_a_test_red_without_test() {
        let mut db = board(State::InReview);
        log(&mut db, "demo.s1", "test-red", &["--", "t_new"]);
        let got = checked(&db, "demo.s1", &added(&["t_new"]), &[]);
        assert!(matches!(got, Err(CommitRefused::TestUnproven { .. })));
    }

    /// GW-E-033 — the argument vector has the template's length: ferrule adds no argument.
    #[test]
    fn ut_argv_adds_no_argument() {
        let c = cmds(vec![cfg(
            None,
            &["run", "--filter={test}", "x{test}y"],
            None,
        )]);
        let Pick::Found { command, bare } = c.pick("a::b") else {
            panic!("no command");
        };
        let argv = command.argv(bare);
        assert_eq!(argv, strings(&["run", "--filter=a::b", "xa::by"]));
    }
}
