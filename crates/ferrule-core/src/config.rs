//! `.ferrule/ferrule.toml`: what differs between projects, read once per
//! command. Every key has a default, so a project without the file works.

use std::path::Path;

use serde::Deserialize;

pub const FILE: &str = "ferrule.toml";

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// The branch runs merge into.
    pub main_branch: String,
    pub stage_stop: StageStop,
    pub gauntlet: Gauntlet,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            main_branch: "main".to_owned(),
            stage_stop: StageStop::default(),
            gauntlet: Gauntlet::default(),
        }
    }
}

/// When a clean full-lane stage stops for the user instead of continuing.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct StageStop {
    /// A stage diff above this many changed lines stops.
    pub lines: usize,
    /// A stage diff touching more files than this stops.
    pub files: usize,
    /// Stages auto-continued in a row before one stops anyway.
    pub checkpoint: usize,
}

impl Default for StageStop {
    fn default() -> Self {
        StageStop {
            lines: 400,
            files: 8,
            checkpoint: 3,
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct Gauntlet {
    /// Run in order in the worktree; the first failure ends the gauntlet.
    pub steps: Vec<Step>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Step {
    /// The word `gauntlet=fail step=<name>` reports.
    pub name: String,
    /// Program and arguments, run without a shell.
    pub run: Vec<String>,
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    /// The step prints a `cargo llvm-cov` summary whose `TOTAL` line holds
    /// the line coverage the pass reports.
    #[serde(default)]
    pub coverage: bool,
}

fn default_timeout() -> u64 {
    1800
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
        let c: Config = toml::from_str("[stage_stop]\nfiles = 2\n").unwrap();
        assert_eq!(c.main_branch, "main");
        assert_eq!(c.stage_stop.files, 2);
        assert_eq!(c.stage_stop.lines, 400);
    }

    #[test]
    fn ut_gauntlet_steps() {
        let c: Config = toml::from_str(
            "[[gauntlet.steps]]\nname = \"fmt\"\nrun = [\"cargo\", \"fmt\", \"--check\"]\ntimeout_secs = 900\n",
        )
        .unwrap();
        assert_eq!(c.gauntlet.steps[0].run, ["cargo", "fmt", "--check"]);
        assert!(!c.gauntlet.steps[0].coverage);
    }

    #[test]
    fn ut_unknown_keys_are_refused() {
        assert!(toml::from_str::<Config>("main = \"x\"\n").is_err());
    }
}
