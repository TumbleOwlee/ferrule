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
    /// Where pull requests and their CI live; `ci`, its former name, is
    /// still read.
    #[serde(alias = "ci")]
    pub host: Host,
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
pub enum Host {
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
    let unit = s.chars().next_back()?;
    let n: u64 = s[..s.len() - unit.len_utf8()].parse().ok()?;
    let secs = match unit {
        's' => n,
        'm' => n.checked_mul(60)?,
        'h' => n.checked_mul(3600)?,
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
    /// Commands the reviewer's guard lets through besides its allowlist, as
    /// word prefixes (`make check` allows `make check ARGS=x`).
    pub allow: Vec<String>,
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
    /// `spec related`, `spec refs` and `issue search` print at most this
    /// many hits; more is refused.
    pub search_max: usize,
    /// How long `ci wait` waits for the PR's checks before giving up.
    #[serde(deserialize_with = "duration")]
    pub ci_wait: Duration,
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
            search_max: 40,
            ci_wait: Duration::from_hours(2),
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

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            Error::Parse(e) => Some(e),
        }
    }
}

impl Config {
    /// `<root>/.ferrule/ferrule.toml`, or the defaults when it is absent.
    pub fn load(root: &Path) -> Result<Self, Error> {
        let path = root.join(crate::project::DIR).join(FILE);
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
host = "github"

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
allow = ["make check"]

[thresholds]
read_budget = 3
stage_stop_lines = 400
stage_stop_files = 8
summary_max_lines = 25
summary_max_bytes = 2048
ci_wait = "45m"
"#,
        )
        .unwrap();
        assert_eq!(c.gauntlet.step[0].timeout, Duration::from_mins(15));
        assert_eq!(c.gauntlet.step[1].coverage, Some(Coverage::LlvmCov));
        assert_eq!(c.viewer.command.as_deref(), Some(&["glow".to_owned()][..]));
        assert_eq!(c.hook.readonly.mutating.len(), 4);
        assert_eq!(c.hook.readonly.allow, ["make check"]);
        assert_eq!(c.backend.tracker, Tracker::Github);
        let old: Config = toml::from_str("[backend]\nci = \"bitbucket\"\n").unwrap();
        assert_eq!(old.backend.host, Host::Bitbucket, "ci is still read");
        assert_eq!(c.thresholds.ci_wait, Duration::from_mins(45));
        assert_eq!(Thresholds::default().ci_wait, Duration::from_hours(2));
    }

    #[test]
    fn ut_durations() {
        assert_eq!(parse_duration("90s"), Some(Duration::from_secs(90)));
        assert_eq!(parse_duration("2h"), Some(Duration::from_hours(2)));
        assert_eq!(parse_duration("15"), None);
        assert_eq!(parse_duration(""), None);
        assert_eq!(parse_duration("5é"), None);
        assert_eq!(parse_duration("é"), None);
        assert_eq!(parse_duration("18446744073709551615h"), None);
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
