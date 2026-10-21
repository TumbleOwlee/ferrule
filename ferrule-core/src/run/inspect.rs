//! What a run looks like from the board, git and its files, read only:
//! the answers of `run status`, `run diff`, `run log` and `run plan-diff`.

use std::path::{Path, PathBuf};

use rusqlite::Connection;

use crate::card::board::{self, State};
use crate::card::typed::Name;
use crate::config::Config;
use crate::git::{self, GitError};
use crate::project::Layout;
use crate::refusal::Refusal;

/// Why an inspection gave no answer: a refusal, or an internal failure.
#[derive(Debug)]
pub enum Error {
    Refused(Box<Refusal>),
    Sql(rusqlite::Error),
    Config(crate::config::Error),
    Git(GitError),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Refused(r) => write!(f, "{r}"),
            Error::Sql(e) => write!(f, "{e}"),
            Error::Config(e) => write!(f, "{e}"),
            Error::Git(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Refused(r) => Some(r.as_ref()),
            Error::Sql(e) => Some(e),
            Error::Config(e) => Some(e),
            Error::Git(e) => Some(e),
        }
    }
}

fn refused(r: Refusal) -> Error {
    Error::Refused(Box::new(r))
}

impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Self {
        Error::Sql(e)
    }
}

impl From<crate::config::Error> for Error {
    fn from(e: crate::config::Error) -> Self {
        Error::Config(e)
    }
}

impl From<GitError> for Error {
    fn from(e: GitError) -> Self {
        Error::Git(e)
    }
}

/// The parent card's line of `run status`; `None` is a field not set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parent {
    pub state: State,
    pub lane: Option<String>,
    pub branch: Option<String>,
    pub issue: Option<String>,
    pub pr: Option<String>,
    /// The last `pr=` logged: draft or ready.
    pub pr_state: Option<String>,
    /// The gates set of gate1, arch, no-arch and gate2, in that order.
    pub gates: Vec<&'static str>,
}

/// One stage card's line of `run status`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageLine {
    pub n: u32,
    pub state: State,
    /// The card's last `commit=`.
    pub commit: Option<String>,
    pub title: Option<String>,
}

/// The run's worktree; `path` is relative to the project root when it
/// lies below it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Worktree {
    Missing {
        path: String,
    },
    Present {
        path: String,
        /// The short sha of HEAD, `None` when git could not tell.
        head: Option<String>,
        /// Commits since `main_branch`, `None` when git could not tell.
        ahead: Option<String>,
        /// Files with uncommitted changes.
        uncommitted: usize,
    },
}

/// A logged line of the run: the card, its time and its body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Logged {
    pub card: String,
    pub at: String,
    pub body: String,
}

/// Where a run stands, local only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    pub parent: Parent,
    pub stages: Vec<StageLine>,
    pub worktree: Worktree,
    /// The parent's last gauntlet line, without `gauntlet=`.
    pub gauntlet: Option<String>,
    /// The parent's last `ci=`.
    pub ci: Option<String>,
    /// The run's last logged line; `None` when it has logged nothing.
    pub last: Option<Logged>,
}

/// Where run `slug` stands; `conn` is the board, `None` when there is none.
///
/// # Errors
/// [`Error::Refused`] `status=refused reason=card` when the run has no
/// card; [`Error::Sql`] when the board cannot be read.
pub fn status(
    conn: Option<&Connection>,
    layout: &Layout,
    main_branch: &str,
    slug: &str,
) -> Result<Status, Error> {
    let found = match conn {
        Some(c) => board::state(c, slug)?.map(|st| (c, st)),
        None => None,
    };
    let Some((c, state)) = found else {
        return Err(refused(Refusal::card("status")));
    };
    let field = |card: &str, key: &str| board::field(c, card, key);
    let mut gates = Vec::new();
    for g in ["gate1", "arch", "no-arch", "gate2"] {
        if field(slug, g)?.is_some() {
            gates.push(g);
        }
    }
    let parent = Parent {
        state,
        lane: field(slug, "lane")?,
        branch: field(slug, "branch")?,
        issue: field(slug, "issue")?,
        pr: field(slug, "pr")?,
        pr_state: board::last_value(c, slug, Name::Pr)?,
        gates,
    };
    let mut stages = Vec::new();
    for (n, id, st) in board::stages(c, slug)? {
        stages.push(StageLine {
            n,
            state: st,
            commit: board::last_value(c, &id, Name::Commit)?,
            title: field(&id, "title")?,
        });
    }
    let wt = super::worktree(c, layout, slug, slug)?;
    let path = wt
        .strip_prefix(&layout.root)
        .unwrap_or(&wt)
        .display()
        .to_string();
    let worktree = if wt.is_dir() {
        let range = format!("{main_branch}..HEAD");
        Worktree::Present {
            path,
            head: git::out(&wt, &["rev-parse", "--short", "HEAD"]),
            ahead: git::out(&wt, &["rev-list", "--count", &range]),
            uncommitted: git::out(&wt, &["status", "--porcelain"]).map_or(0, |o| o.lines().count()),
        }
    } else {
        Worktree::Missing { path }
    };
    let gauntlet = board::log_lines(c, slug, Some("gauntlet="))?
        .pop()
        .and_then(|(_, body)| body.strip_prefix("gauntlet=").map(str::to_owned));
    let last = board::run_log_lines(c, slug, None)?
        .pop()
        .map(|(card, at, body)| Logged { card, at, body });
    Ok(Status {
        parent,
        stages,
        worktree,
        gauntlet,
        ci: board::last_value(c, slug, Name::Ci)?,
        last,
    })
}

/// What `run diff` and `run log` show.
#[derive(Debug, Clone, Copy)]
pub struct Pick<'a> {
    /// Pathspecs the diff is narrowed to; `run log` ignores them.
    pub paths: &'a [String],
    pub stat: bool,
    /// The commit to compare with [default: `main_branch` of ferrule.toml].
    pub base: Option<&'a str>,
}

/// Git commands to run in `dir`, in order; their output, whole and
/// unchanged, is the answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invocation {
    pub dir: PathBuf,
    pub calls: Vec<Vec<String>>,
}

/// The git commands that print the run branch's diff: since it left the
/// base, or with `uncommitted` the worktree's status and its changes
/// against HEAD.
///
/// # Errors
/// [`Error::Refused`] `diff=refused` with reason card, no-worktree or base;
/// an internal [`Error`] when the board or ferrule.toml cannot be read.
pub fn diff(
    conn: Option<&Connection>,
    layout: &Layout,
    slug: &str,
    pick: &Pick,
    uncommitted: bool,
) -> Result<Invocation, Error> {
    let dir = run_worktree("diff", conn, layout, slug)?;
    let mut calls = Vec::new();
    let mut args = vec!["diff", "--no-color", "--no-ext-diff"];
    if pick.stat {
        args.push("--stat");
    }
    let range;
    if uncommitted {
        calls.push(owned(&["status", "--short"]));
        args.push("HEAD");
    } else {
        range = format!("{}...HEAD", base_in("diff", layout, &dir, pick.base)?);
        args.push(&range);
    }
    args.push("--");
    args.extend(pick.paths.iter().map(String::as_str));
    calls.push(owned(&args));
    Ok(Invocation { dir, calls })
}

/// The git command that prints the run branch's commits since the base.
///
/// # Errors
/// [`Error::Refused`] `log=refused` with reason card, no-worktree or base;
/// an internal [`Error`] when the board or ferrule.toml cannot be read.
pub fn log(
    conn: Option<&Connection>,
    layout: &Layout,
    slug: &str,
    pick: &Pick,
) -> Result<Invocation, Error> {
    let dir = run_worktree("log", conn, layout, slug)?;
    let range = format!("{}..HEAD", base_in("log", layout, &dir, pick.base)?);
    let mut args = vec!["log", "--no-color", "--oneline"];
    if pick.stat {
        args.push("--stat");
    }
    args.push(&range);
    let calls = vec![owned(&args)];
    Ok(Invocation { dir, calls })
}

/// The unified diff from plan.prev.md to plan.md in the run's directory;
/// empty when plan.md has not changed.
///
/// # Errors
/// [`Error::Refused`] `plan-diff=refused` with reason no-plan or no-prev;
/// [`Error::Git`] when git fails.
pub fn plan_diff(layout: &Layout, slug: &str) -> Result<Vec<u8>, Error> {
    let dir = layout.run_dir(slug);
    if !dir.join("plan.md").is_file() {
        let r =
            Refusal::new("plan-diff", "no-plan").hint("the planner has not written plan.md yet");
        return Err(refused(r));
    }
    if !dir.join("plan.prev.md").is_file() {
        let r = Refusal::new("plan-diff", "no-prev")
            .hint("no plan review has snapshotted plan.md; read it whole");
        return Err(refused(r));
    }
    let args = [
        "diff",
        "--no-index",
        "--no-color",
        "--no-ext-diff",
        "--",
        "plan.prev.md",
        "plan.md",
    ];
    Ok(git::bytes(&dir, &args, &[1])?)
}

fn owned(args: &[&str]) -> Vec<String> {
    args.iter().map(|&a| a.to_owned()).collect()
}

/// The run's worktree, or the `<verb>` refusal of why there is none.
fn run_worktree(
    verb: &'static str,
    conn: Option<&Connection>,
    layout: &Layout,
    slug: &str,
) -> Result<PathBuf, Error> {
    let c = match conn {
        Some(c) if board::state(c, slug)?.is_some() => c,
        _ => return Err(refused(Refusal::card(verb))),
    };
    let wt = super::worktree(c, layout, slug, slug)?;
    if !wt.is_dir() {
        return Err(refused(
            Refusal::new(verb, "no-worktree")
                .hint(format!("`ferrule gate approve {slug} gate2` adds it")),
        ));
    }
    Ok(wt)
}

/// The commit `base` names in `wt` (`main_branch` by default), or the
/// `<verb>` refusal of why it names none.
fn base_in(
    verb: &'static str,
    layout: &Layout,
    wt: &Path,
    base: Option<&str>,
) -> Result<String, Error> {
    let base = match base {
        Some(b) => b.to_owned(),
        None => Config::load(&layout.root)?.main_branch,
    };
    if git::commit(wt, &base).is_none() {
        return Err(refused(
            Refusal::new(verb, "base").hint("fetch it, or pass a --base the branch has"),
        ));
    }
    Ok(base)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;

    fn git(dir: &Path, args: &[&str]) {
        let ok = git::ok(
            dir,
            &[
                &[
                    "-c",
                    "user.name=t",
                    "-c",
                    "user.email=t@t",
                    "-c",
                    "commit.gpgsign=false",
                ],
                args,
            ]
            .concat(),
        );
        assert!(ok, "git {args:?}");
    }

    /// A project root on main with one commit, and the run `r`'s worktree
    /// one commit ahead on feat/r.
    fn scratch() -> (tempfile::TempDir, Layout) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        git(&root, &["init", "-q", "-b", "main"]);
        std::fs::write(root.join("a"), "a\n").unwrap();
        git(&root, &["add", "a"]);
        git(&root, &["commit", "-q", "-m", "a"]);
        let layout = Layout::new(root.clone());
        let wt = layout.worktree("r");
        git(
            &root,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "feat/r",
                wt.to_str().unwrap(),
            ],
        );
        std::fs::write(wt.join("b"), "b\n").unwrap();
        git(&wt, &["add", "b"]);
        git(&wt, &["commit", "-q", "-m", "b"]);
        (dir, layout)
    }

    fn f(k: &str, v: &str) -> (String, String) {
        (k.to_owned(), v.to_owned())
    }

    #[test]
    fn ut_status_reads_cards_worktree_and_log() {
        let (_dir, layout) = scratch();
        let wt = layout.worktree("r");
        std::fs::write(wt.join("a"), "changed\n").unwrap();
        std::fs::write(wt.join("c"), "new\n").unwrap();
        let mut s = Store::memory().unwrap();
        board::create(
            &mut s.conn,
            "r",
            State::InProgress,
            &[f("lane", "full"), f("branch", "feat/r"), f("gate1", "d")],
            "t",
        )
        .unwrap();
        board::create(&mut s.conn, "r.s1", State::Done, &[f("title", "one")], "t").unwrap();
        board::create(&mut s.conn, "r.s2", State::Open, &[f("title", "two")], "t").unwrap();
        board::insert_body(&s.conn, "r.s1", "t1", "commit=abc1234");
        board::insert_body(&s.conn, "r", "t2", "gauntlet=pass cov=? sha=abc1234");
        board::insert_body(&s.conn, "r", "t3", "ci=0");

        let st = status(Some(&s.conn), &layout, "main", "r").unwrap();
        assert_eq!(
            st.parent,
            Parent {
                state: State::InProgress,
                lane: Some("full".into()),
                branch: Some("feat/r".into()),
                issue: None,
                pr: None,
                pr_state: None,
                gates: vec!["gate1"],
            }
        );
        assert_eq!(
            st.stages,
            [
                StageLine {
                    n: 1,
                    state: State::Done,
                    commit: Some("abc1234".into()),
                    title: Some("one".into()),
                },
                StageLine {
                    n: 2,
                    state: State::Open,
                    commit: None,
                    title: Some("two".into()),
                },
            ]
        );
        let head = git::out(&wt, &["rev-parse", "--short", "HEAD"]);
        assert_eq!(
            st.worktree,
            Worktree::Present {
                path: ".ferrule/worktrees/r".into(),
                head,
                ahead: Some("1".into()),
                uncommitted: 2,
            }
        );
        assert_eq!(st.gauntlet.as_deref(), Some("pass cov=? sha=abc1234"));
        assert_eq!(st.ci.as_deref(), Some("0"));
        assert_eq!(
            st.last,
            Some(Logged {
                card: "r".into(),
                at: "t3".into(),
                body: "ci=0".into(),
            })
        );
    }

    #[test]
    fn ut_status_refuses_a_missing_run_and_shows_a_missing_worktree() {
        let layout = Layout::new(PathBuf::from("/nonexistent"));
        let r = status(None, &layout, "main", "r").unwrap_err();
        assert!(
            matches!(&r, Error::Refused(r) if r.to_string().starts_with("status=refused reason=card"))
        );
        let mut s = Store::memory().unwrap();
        let r = status(Some(&s.conn), &layout, "main", "r").unwrap_err();
        assert!(matches!(r, Error::Refused(_)));
        board::create(&mut s.conn, "r", State::Open, &[], "t").unwrap();
        let st = status(Some(&s.conn), &layout, "main", "r").unwrap();
        assert_eq!(
            st.worktree,
            Worktree::Missing {
                path: ".ferrule/worktrees/r".into()
            }
        );
        assert_eq!((st.gauntlet, st.ci, st.last), (None, None, None));
    }

    #[test]
    fn ut_diff_and_log_name_the_git_commands_or_refuse() {
        let (_dir, layout) = scratch();
        let mut s = Store::memory().unwrap();
        board::create(&mut s.conn, "r", State::InProgress, &[], "t").unwrap();
        let wt = layout.worktree("r");
        let paths = ["docs/specs".to_owned()];
        let pick = Pick {
            paths: &paths,
            stat: true,
            base: None,
        };
        let inv = diff(Some(&s.conn), &layout, "r", &pick, false).unwrap();
        assert_eq!(inv.dir, wt);
        assert_eq!(
            inv.calls,
            [owned(&[
                "diff",
                "--no-color",
                "--no-ext-diff",
                "--stat",
                "main...HEAD",
                "--",
                "docs/specs"
            ])]
        );
        let inv = diff(Some(&s.conn), &layout, "r", &pick, true).unwrap();
        assert_eq!(inv.calls[0], owned(&["status", "--short"]));
        assert_eq!(
            inv.calls[1][3..],
            owned(&["--stat", "HEAD", "--", "docs/specs"])
        );
        let lines = log(Some(&s.conn), &layout, "r", &pick).unwrap();
        assert_eq!(
            lines.calls,
            [owned(&[
                "log",
                "--no-color",
                "--oneline",
                "--stat",
                "main..HEAD"
            ])]
        );
        let bad = Pick {
            base: Some("nope"),
            ..pick
        };
        let e = log(Some(&s.conn), &layout, "r", &bad).unwrap_err();
        assert!(matches!(&e, Error::Refused(r) if r.verb() == "log" && r.reason() == "base"));
        let e = diff(Some(&s.conn), &layout, "x", &pick, false).unwrap_err();
        assert!(matches!(&e, Error::Refused(r) if r.reason() == "card"));
        board::create(&mut s.conn, "x", State::Open, &[], "t").unwrap();
        let e = diff(Some(&s.conn), &layout, "x", &pick, false).unwrap_err();
        assert!(matches!(&e, Error::Refused(r) if r.reason() == "no-worktree"));
    }

    #[test]
    fn ut_plan_diff_is_empty_when_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let layout = Layout::new(dir.path().to_path_buf());
        let e = plan_diff(&layout, "r").unwrap_err();
        assert!(matches!(&e, Error::Refused(r) if r.reason() == "no-plan"));
        let run = layout.run_dir("r");
        std::fs::create_dir_all(&run).unwrap();
        std::fs::write(run.join("plan.md"), "one\n").unwrap();
        let e = plan_diff(&layout, "r").unwrap_err();
        assert!(matches!(&e, Error::Refused(r) if r.reason() == "no-prev"));
        std::fs::write(run.join("plan.prev.md"), "one\n").unwrap();
        assert!(plan_diff(&layout, "r").unwrap().is_empty());
        std::fs::write(run.join("plan.md"), "two\n").unwrap();
        let text = String::from_utf8(plan_diff(&layout, "r").unwrap()).unwrap();
        assert!(text.contains("-one\n+two\n"), "{text}");
    }
}
