//! `ci wait`: the PR's checks waited for, the failed jobs' lines on a
//! failure, and the run's next step decided on what they came to.

use crate::config::Config;
use crate::pr::publish::{self, Run};
use crate::project::Layout;
use crate::refusal::Refusal;
use crate::remote;
use crate::run::next::{self, At, Status};
use crate::store::Board;

/// What the wait came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Waited {
    /// The checks' exit code: 0 green.
    pub code: i32,
    /// The failed jobs' names.
    pub failed: Vec<String>,
    /// The failed jobs' log lines, when the host reads them.
    pub lines: Vec<String>,
    /// The next step's lines.
    pub next: Vec<String>,
}

/// Why the wait did not come to an answer.
#[derive(Debug)]
pub enum Error {
    /// No PR is recorded for the run.
    NoPr,
    /// The PR reports no checks yet.
    NoChecks,
    /// Waiting failed otherwise.
    Publish(publish::Error),
    /// Deciding the next step failed.
    Next(next::Error),
}

impl Error {
    /// The `ci=refused` line of a refusal; `None` for other failures.
    #[must_use]
    pub fn refusal(&self, slug: &str) -> Option<Refusal> {
        match self {
            Error::NoPr => Some(Refusal::new("ci", "no-pr").hint(format!(
                "no PR is recorded for this run; open it with `ferrule pr open {slug}`"
            ))),
            Error::NoChecks => Some(Refusal::new("ci", "no-checks").hint(format!(
                "the PR reports no checks yet; rerun `ferrule ci wait {slug}` once CI has started"
            ))),
            Error::Publish(_) | Error::Next(_) => None,
        }
    }
}

/// Waits for run `slug`'s checks and decides its next step, stamped with
/// `now()` taken once the checks finished, not when the wait began.
///
/// # Errors
/// [`Error`].
pub fn wait(
    db: &mut Board,
    layout: &Layout,
    config: &Config,
    slug: &str,
    now: impl Fn() -> String,
) -> Result<Waited, Error> {
    let began = now();
    let waited = Run::new(db, layout, config, &layout.root, slug, &began).wait_ci();
    let checks = match waited {
        Ok(c) => c,
        Err(publish::Error::NoPr) => return Err(Error::NoPr),
        Err(publish::Error::Remote(remote::Error::NoChecks(_))) => return Err(Error::NoChecks),
        Err(e) => return Err(Error::Publish(e)),
    };
    let finished = now();
    let mut lines = Vec::new();
    if checks.code != 0 {
        let branch = db
            .read(|r| r.field(slug, "branch"))
            .map_err(|e| Error::Publish(publish::Error::Sql(e)))?
            .unwrap_or_default();
        if let Ok(found) =
            remote::code_host(config, &layout.root).failed(&branch, remote::Query::default())
        {
            lines = found;
        }
    }
    let status = Status::parse(&[format!("ci={}", checks.code)]);
    let next = next::next(db, layout, slug, At::Ci, &status, &finished).map_err(Error::Next)?;
    Ok(Waited {
        code: checks.code,
        failed: checks.failed,
        lines,
        next,
    })
}
