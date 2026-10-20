//! `.ferrule/ferrule.toml`: what differs between projects, read once per
//! command. Every key has a default, so a project without the file works;
//! an unknown key is refused.

use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Deserializer};

pub const FILE: &str = "ferrule.toml";

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// The ferrule version the project is pinned to.
    pub version: Option<String>,
    /// Agent hosts `ferrule setup install` renders for.
    pub hosts: Vec<String>,
    /// The project's stack, detected at install.
    pub stack: Option<String>,
    /// The branch runs merge into.
    pub main_branch: String,
    pub backend: Backend,
    pub gauntlet: Gauntlet,
    pub viewer: Viewer,
    pub hook: Hook,
    pub guard: Guard,
    pub thresholds: Thresholds,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            version: None,
            hosts: vec!["claude".to_owned()],
            stack: None,
            main_branch: "main".to_owned(),
            backend: Backend::default(),
            gauntlet: Gauntlet::default(),
            viewer: Viewer::default(),
            hook: Hook::default(),
            guard: Guard::default(),
            thresholds: Thresholds::default(),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct Backend {
    /// Where issues live.
    pub tracker: Tracker,
    /// Where CI runs and pull requests live.
    pub ci: Ci,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Tracker {
    /// Through `gh`.
    #[default]
    Github,
    /// Through the REST API, credentials in `.ferrule/jira.local.json`.
    Jira,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Ci {
    /// Through `gh`.
    #[default]
    Github,
    /// Through the REST API, credentials in `.ferrule/bitbucket.local.json`.
    Bitbucket,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct Gauntlet {
    /// Run in order in the worktree; the first failure ends the gauntlet.
    pub step: Vec<Step>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Step {
    /// The word `gauntlet=fail step=<name>` reports.
    pub name: String,
    /// Program and arguments, run without a shell.
    pub cmd: Vec<String>,
    /// `90s`, `15m`, `1h`.
    #[serde(default = "default_timeout", deserialize_with = "duration")]
    pub timeout: Duration,
    /// How the pass's `cov=` is read from this step's output.
    #[serde(default)]
    pub coverage: Option<Coverage>,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Coverage {
    /// The third percentage on `cargo llvm-cov`'s last `TOTAL` line.
    LlvmCov,
}

fn default_timeout() -> Duration {
    Duration::from_mins(30)
}

/// `<n>s`, `<n>m` or `<n>h`.
fn parse_duration(s: &str) -> Option<Duration> {
    let (n, unit) = s.split_at(s.len().checked_sub(1)?);
    let n: u64 = n.parse().ok()?;
    let secs = match unit {
        "s" => n,
        "m" => n * 60,
        "h" => n * 3600,
        _ => return None,
    };
    Some(Duration::from_secs(secs))
}

fn duration<'de, D: Deserializer<'de>>(d: D) -> Result<Duration, D::Error> {
    let s = String::deserialize(d)?;
    parse_duration(&s)
        .ok_or_else(|| serde::de::Error::custom(format!("expected <n>s, <n>m or <n>h, got {s}")))
}

/// Where `prompt show` opens files.
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct Viewer {
    /// Program and arguments; each file's path is appended. Unset: the
    /// files are printed to stdout.
    pub command: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct Hook {
    pub readonly: Readonly,
}

/// The command guard of every session.
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct Guard {
    /// Raw commands the project re-opens although ferrule replaces them,
    /// as word prefixes (`gh api`).
    pub allow: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct Readonly {
    /// Commands the reviewer's guard refuses besides its own list, as word
    /// prefixes (`cargo add` refuses `cargo add serde`).
    pub mutating: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct Thresholds {
    /// `read` events one stage card may carry.
    pub read_budget: usize,
    /// A stage diff above this many changed lines stops.
    pub stage_stop_lines: usize,
    /// A stage diff touching more files than this stops.
    pub stage_stop_files: usize,
    /// Stages auto-continued in a row before one stops anyway.
    pub stage_stop_checkpoint: usize,
    /// A user-facing summary longer than this is refused.
    pub summary_max_lines: usize,
    pub summary_max_bytes: usize,
    /// `spec related` lists at most this many entries; more is refused.
    pub related_max: usize,
}

impl Default for Thresholds {
    fn default() -> Self {
        Thresholds {
            read_budget: 3,
            stage_stop_lines: 400,
            stage_stop_files: 8,
            stage_stop_checkpoint: 3,
            summary_max_lines: 25,
            summary_max_bytes: 2048,
            related_max: 40,
        }
    }
}

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    Parse(toml::de::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Io(e) => write!(f, "{FILE}: {e}"),
            Error::Parse(e) => write!(f, "{FILE}: {e}"),
        }
    }
}

impl std::error::Error for Error {}

impl Config {
    /// `<root>/.ferrule/ferrule.toml`, or the defaults when it is absent.
    pub fn load(root: &Path) -> Result<Self, Error> {
        let path = root.join(crate::store::DIR).join(FILE);
        match std::fs::read_to_string(&path) {
            Ok(text) => toml::from_str(&text).map_err(Error::Parse),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
            Err(e) => Err(Error::Io(e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ut_defaults_and_partial_files() {
        let c: Config = toml::from_str("[thresholds]\nstage_stop_files = 2\n").unwrap();
        assert_eq!(c.main_branch, "main");
        assert_eq!(c.thresholds.stage_stop_files, 2);
        assert_eq!(c.thresholds.stage_stop_lines, 400);
        assert_eq!(c.viewer.command, None);
    }

    #[test]
    fn ut_the_designs_example_parses() {
        let c: Config = toml::from_str(
            r#"
version = "0.4.2"
hosts = ["claude"]
stack = "rust"

[backend]
tracker = "github"
ci = "github"

[[gauntlet.step]]
name = "fmt"
cmd = ["cargo", "fmt", "--check"]
timeout = "15m"

[[gauntlet.step]]
name = "cov"
cmd = ["cargo", "llvm-cov", "--workspace", "--fail-under-lines", "80"]
timeout = "30m"
coverage = "llvm-cov"

[viewer]
command = ["glow"]

[hook.readonly]
mutating = ["cargo add", "cargo remove", "cargo install", "cargo update"]

[thresholds]
read_budget = 3
stage_stop_lines = 400
stage_stop_files = 8
summary_max_lines = 25
summary_max_bytes = 2048
"#,
        )
        .unwrap();
        assert_eq!(c.gauntlet.step[0].timeout, Duration::from_mins(15));
        assert_eq!(c.gauntlet.step[1].coverage, Some(Coverage::LlvmCov));
        assert_eq!(c.viewer.command.as_deref(), Some(&["glow".to_owned()][..]));
        assert_eq!(c.hook.readonly.mutating.len(), 4);
        assert_eq!(c.backend.tracker, Tracker::Github);
    }

    #[test]
    fn ut_durations() {
        assert_eq!(parse_duration("90s"), Some(Duration::from_secs(90)));
        assert_eq!(parse_duration("2h"), Some(Duration::from_hours(2)));
        assert_eq!(parse_duration("15"), None);
        assert_eq!(parse_duration(""), None);
    }

    #[test]
    fn ut_unknown_keys_are_refused() {
        assert!(toml::from_str::<Config>("main = \"x\"\n").is_err());
        assert!(toml::from_str::<Config>("[backend]\ntracker = \"gitlab\"\n").is_err());
        assert!(
            toml::from_str::<Config>(
                "[[gauntlet.step]]\nname = \"a\"\ncmd = []\ntimeout = \"soon\"\n"
            )
            .is_err()
        );
    }
}
