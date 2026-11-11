//! The run's preflight: on GitHub, `gh` is logged in, and who it is.

use std::path::Path;

use crate::config::{Config, Host};
use crate::remote::{self, Error};

/// What the preflight found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Preflight {
    /// `gh` is logged in as `login`.
    Ok { login: String },
    /// The code host is not GitHub: no check.
    Skipped { host: &'static str },
    /// `gh` is not logged in.
    LoggedOut,
    /// `gh` did not start or did not answer: its error.
    Unknown(String),
}

/// Reads a failed `gh auth status` message.
fn classify(msg: &str) -> Preflight {
    let lower = msg.to_lowercase();
    if lower.contains("not logged in") || lower.contains("gh auth login") {
        Preflight::LoggedOut
    } else {
        Preflight::Unknown(msg.to_owned())
    }
}

fn message(e: &Error) -> String {
    match e {
        Error::Command(m) => m.clone(),
        other => other.to_string(),
    }
}

/// Runs `gh auth status`, then `gh api user --jq .login`, in `root`.
#[must_use]
pub fn check(config: &Config, root: &Path) -> Preflight {
    match config.backend.host {
        Host::Github => {}
        Host::Bitbucket => return Preflight::Skipped { host: "bitbucket" },
    }
    match remote::run("gh", &["auth", "status"], None, root) {
        Ok(_) => {}
        Err(Error::Command(msg)) => return classify(&msg),
        Err(e) => return Preflight::Unknown(message(&e)),
    }
    match remote::run("gh", &["api", "user", "--jq", ".login"], None, root) {
        Ok(out) if !out.trim().is_empty() => Preflight::Ok {
            login: out.trim().to_owned(),
        },
        Ok(_) => Preflight::Unknown("gh api user returned no login".to_owned()),
        Err(e) => Preflight::Unknown(message(&e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RU-E-037 — a host other than GitHub is skipped without running `gh`.
    #[test]
    fn ut_check_skips_a_host_other_than_github() {
        let config: Config = toml::from_str("[backend]\nhost = \"bitbucket\"\n").unwrap();
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            check(&config, dir.path()),
            Preflight::Skipped { host: "bitbucket" }
        );
    }

    /// RU-E-035, RU-E-036 — the logged-out message classifies as logged out, any other as unknown.
    #[test]
    fn ut_classify_reads_the_logged_out_message() {
        assert_eq!(
            classify("You are not logged into any GitHub hosts. To log in, run: gh auth login"),
            Preflight::LoggedOut
        );
        assert!(matches!(
            classify("gh: No such file or directory (os error 2)"),
            Preflight::Unknown(_)
        ));
    }
}
