//! `stage commit`: a commit on the run's branch, never on the main branch,
//! of a stage card in review, with `commit=<sha>` logged on the card.

use std::path::Path;

use crate::card::board::{self, CardId, State};
use crate::git;
use crate::hook::attribution;
use crate::refusal::Refusal;
use crate::store::Board;

/// Why nothing was committed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommitRefused {
    /// HEAD is on the main branch, named.
    Main(String),
    /// No board, or no such card.
    Card,
    /// The card is not in review: its state.
    State(State),
    /// HEAD is not the run's branch, named.
    Branch(String),
    /// `git commit` failed.
    Git,
}

impl CommitRefused {
    /// The `commit=refused` line; `card` is the stage card asked for.
    #[must_use]
    pub fn refusal(&self, card: Option<&str>) -> Refusal {
        match self {
            CommitRefused::Main(main) => Refusal::new("commit", "main")
                .field("branch", main)
                .hint(format!(
                    "commit on the run's branch, not {main}: `ferrule gate approve <slug> gate2` creates it and its worktree; with --no-run, check out another branch first"
                )),
            CommitRefused::Card => Refusal::card("commit"),
            CommitRefused::State(st) => Refusal::new("commit", "state")
                .field("state", st)
                .hint(format!(
                    "move the card to inreview first: `ferrule board move {} inreview`",
                    card.unwrap_or("<card>")
                )),
            CommitRefused::Branch(want) => Refusal::new("commit", "branch")
                .field("want", want)
                .hint(format!(
                    "check out {want}, the run's branch, and commit there"
                )),
            CommitRefused::Git => Refusal::new("commit", "git").hint(
                "git commit failed; stage the changes with `git add` and read the message git prints on stderr",
            ),
        }
    }
}

/// A commit made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Committed {
    /// Its short sha.
    pub sha: String,
    /// The card `commit=` was logged on, or why it could not be.
    pub logged: Option<Result<String, String>>,
}

/// Commits the staged changes in `here` with `message` (attribution
/// lines stripped). With `card`, the card must be a stage in review on a
/// board and HEAD the run's branch; `commit=<sha>` is then logged on it.
///
/// # Errors
/// The board could not be read.
pub fn commit(
    db: Option<&mut Board>,
    here: &Path,
    main: &str,
    card: Option<&str>,
    message: &str,
    now: &str,
) -> rusqlite::Result<Result<Committed, CommitRefused>> {
    let branch = git::out(here, &["symbolic-ref", "--short", "-q", "HEAD"]);
    if branch.as_deref() == Some(main) {
        return Ok(Err(CommitRefused::Main(main.to_owned())));
    }
    let mut on = None;
    if let Some(id) = card {
        let Some(db) = db else {
            return Ok(Err(CommitRefused::Card));
        };
        let run = CardId::parse(id).run;
        let (state, want) =
            db.read(|r| Ok::<_, rusqlite::Error>((r.state(id)?, r.field(run, "branch")?)))?;
        match state {
            None => return Ok(Err(CommitRefused::Card)),
            Some(State::InReview) => {}
            Some(other) => return Ok(Err(CommitRefused::State(other))),
        }
        if let Some(want) = want.filter(|b| !b.is_empty())
            && branch.as_deref() != Some(want.as_str())
        {
            return Ok(Err(CommitRefused::Branch(want)));
        }
        on = Some((db, id));
    }
    let text = attribution::strip(message).unwrap_or_else(|| message.to_owned());
    let dir = here.to_string_lossy();
    let args = ["-C", &dir, "commit", "-q", "-F", "-"];
    if crate::remote::run("git", &args, Some(&format!("{text}\n")), Path::new("")).is_err() {
        return Ok(Err(CommitRefused::Git));
    }
    let sha = git::out(here, &["rev-parse", "--short", "HEAD"]).unwrap_or_default();
    let logged = on.map(|(db, id)| {
        db.write(|w| w.log(id, &format!("commit={sha}"), &[], now))
            .map(|()| id.to_owned())
            .map_err(|e: board::LogError| e.to_string())
    });
    Ok(Ok(Committed { sha, logged }))
}
