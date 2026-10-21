//! A run's lost board recreated from its issue, its draft PR and git, so the
//! run resumes through the usual reconcile. plan.md is never rebuilt.
//!
//! The draft PR's `## Stages` lines carry the stage ids in HTML comments
//! (`<!-- s1 blocked-by s0 -->`); a ticked line is a landed stage with its
//! commit in parentheses.

use std::collections::HashSet;
use std::fmt::Write as _;
use std::path::Path;

use crate::card::board::{self, NewCard, State};
use crate::git;
use crate::project::Layout;
use crate::refusal::Refusal;
use crate::remote::Item;
use crate::store::Board;

/// Why nothing was written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refused {
    NoPr,
    NoStages,
    NoBranch,
    BoardExists,
    NoIssue,
    /// A ticked stage's commit is not in the repository.
    Sha(String),
    /// A ticked stage names no commit.
    NoSha(String),
    /// The branch is not `<prefix>/<slug>` with a valid slug.
    BadBranch(String),
    /// A `## Stages` line names a stage that is not `s<n>`, one named
    /// twice, or blocks on one that is not `s<n>`.
    BadStage(String),
    Worktree(git::GitError),
}

impl Refused {
    /// The `rebuild=refused` line.
    #[must_use]
    pub fn refusal(&self) -> Refusal {
        let reason = match self {
            Refused::NoPr => "no-pr",
            Refused::NoStages => "no-stages",
            Refused::NoBranch => "no-branch",
            Refused::BoardExists => "board-exists",
            Refused::NoIssue => "no-issue",
            Refused::Sha(_) => "sha",
            Refused::NoSha(_) => "no-sha",
            Refused::BadBranch(_) => "bad-branch",
            Refused::BadStage(_) => "bad-stage",
            Refused::Worktree(_) => "worktree",
        };
        let hint = match self {
            Refused::NoPr => "check the PR number and your host access",
            Refused::NoStages => {
                "the draft PR body needs a `## Stages` list of `- [ ] <title> <!-- s1 -->` lines"
            }
            Refused::NoBranch => "push the PR's branch to origin, then retry",
            Refused::BoardExists => "resume it with `ferrule run reconcile <slug>` instead",
            Refused::NoIssue => "check the tracking issue number",
            Refused::Sha(_) => "fetch the branch so the ticked stage's commit exists",
            Refused::NoSha(_) => {
                "tick the stage as `- [x] <title> (<sha>) <!-- s<n> -->` with its commit, or untick it"
            }
            Refused::BadBranch(_) => {
                "name the branch `<prefix>/<slug>`, the slug letters, digits, -, _ and single dots, e.g. feat/<slug>"
            }
            Refused::BadStage(_) => {
                "mark each `## Stages` line once with `<!-- s<n> -->`, blocking only on `s<n>` stages"
            }
            Refused::Worktree(_) => "git could not add the worktree; check `git worktree list`",
        };
        let mut r = Refusal::new("rebuild", reason);
        if let Refused::Sha(sha) = self {
            r = r.field("sha", sha);
        }
        if let Refused::NoSha(stage) | Refused::BadStage(stage) = self {
            r = r.field("stage", stage);
        }
        if let Refused::BadBranch(branch) = self {
            r = r.field("branch", branch);
        }
        if let Refused::Worktree(e) = self {
            r = r.detail(&e.stderr);
        }
        r.hint(hint)
    }
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.refusal())
    }
}

#[derive(Debug)]
pub enum Error {
    Refused(Refused),
    Sql(rusqlite::Error),
    Board(board::BatchError),
    Io(std::io::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Refused(r) => write!(f, "{r}"),
            Error::Sql(e) => write!(f, "state database: {e}"),
            Error::Board(e) => write!(f, "{e}"),
            Error::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Refused(_) => None,
            Error::Sql(e) => Some(e),
            Error::Board(e) => Some(e),
            Error::Io(e) => Some(e),
        }
    }
}

impl From<Refused> for Error {
    fn from(r: Refused) -> Self {
        Error::Refused(r)
    }
}

impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Self {
        Error::Sql(e)
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

/// Where the run's issue, PR and branch come from; each asked only when
/// the rebuild gets that far.
pub trait Source {
    fn pr(&self) -> Option<Item>;
    /// The PR's head branch.
    fn branch(&self) -> Option<String>;
    fn issue(&self) -> Option<Item>;
}

/// One `## Stages` line.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Stage {
    sid: String,
    done: bool,
    blocked_by: Vec<String>,
    sha: Option<String>,
    title: String,
    /// `beyond unit tests:` text and its sub-lines.
    verification: Option<(String, Vec<String>)>,
}

/// The lines under `## <name>` up to the next heading or `Closes #`.
fn section<'a>(body: &'a str, name: &str) -> Vec<&'a str> {
    let heading = format!("## {name}");
    body.lines()
        .skip_while(|l| *l != heading)
        .skip(1)
        .take_while(|l| !l.starts_with("## ") && !l.starts_with("Closes #"))
        .collect()
}

fn trim_blank_end(mut v: Vec<&str>) -> Vec<&str> {
    while v.last().is_some_and(|l| l.is_empty()) {
        v.pop();
    }
    v
}

fn stages(pr: &str) -> Vec<Stage> {
    let mut out: Vec<Stage> = Vec::new();
    for line in section(pr, "Stages") {
        if let Some(rest) = line.strip_prefix("  - beyond unit tests: ") {
            if let Some(s) = out.last_mut() {
                s.verification = Some((rest.to_owned(), Vec::new()));
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("    - ") {
            if let Some((_, subs)) = out.last_mut().and_then(|s| s.verification.as_mut()) {
                subs.push(rest.to_owned());
            }
            continue;
        }
        let done = if line.starts_with("- [x] ") {
            true
        } else if line.starts_with("- [ ] ") {
            false
        } else {
            continue;
        };
        let Some(start) = line.find("<!-- s") else {
            continue;
        };
        let Some(end) = line[start..].find(" -->") else {
            continue;
        };
        let mut words = line[start + 5..start + end].split(' ');
        let sid = words.next().unwrap_or("").to_owned();
        let blocked_by = match (words.next(), words.next()) {
            (Some("blocked-by"), Some(list)) => list.split(',').map(str::to_owned).collect(),
            _ => Vec::new(),
        };
        let mut title = line[6..start].trim_end();
        if let Some(i) = title.find(" — ") {
            title = &title[..i];
        }
        let mut sha = None;
        if let Some(open) = title.rfind(" (")
            && title.ends_with(')')
            && title[open + 2..title.len() - 1]
                .bytes()
                .all(|b| b.is_ascii_hexdigit())
            && title.len() > open + 3
        {
            sha = Some(title[open + 2..title.len() - 1].to_owned());
            title = &title[..open];
        }
        out.push(Stage {
            sid,
            done,
            blocked_by,
            sha,
            title: title.to_owned(),
            verification: None,
        });
    }
    out
}

/// `**ID** — text` or `| **ID** | a | b |`, as `(id, text)`.
fn entry(line: &str) -> Option<(String, String)> {
    let l = line.strip_prefix("- ").unwrap_or(line);
    if let Some(rest) = l.strip_prefix("**")
        && let Some((id, after)) = rest.split_once("** — ")
        && crate::spec::fixed_id_len(id.as_bytes()) == Some(id.len())
    {
        return Some((id.to_owned(), after.to_owned()));
    }
    let cells: Vec<&str> = l.split('|').collect();
    if l.starts_with("| **") && cells.len() > 2 {
        let id = cells[1].trim().trim_matches('*');
        if crate::spec::fixed_id_len(id.as_bytes()) == Some(id.len()) {
            let text: Vec<&str> = cells[2..cells.len() - 1].iter().map(|c| c.trim()).collect();
            return Some((id.to_owned(), text.join(" — ")));
        }
    }
    None
}

/// The spec entries the branch adds, changes or drops, file by file.
fn spec_entries(root: &Path, main: &str, branch: &str) -> Vec<String> {
    let range = format!("{main}...origin/{branch}");
    let files =
        git::out(root, &["diff", "--name-only", &range, "--", "docs/specs"]).unwrap_or_default();
    let mut out = Vec::new();
    for f in files.lines().filter(|f| !f.is_empty()) {
        let diff = git::out(root, &["diff", "-U0", &range, "--", f]).unwrap_or_default();
        let mut order: Vec<String> = Vec::new();
        let mut plus: Vec<(String, String)> = Vec::new();
        let mut minus: Vec<(String, String)> = Vec::new();
        for l in diff.lines() {
            if l.starts_with("+++") || l.starts_with("---") {
                continue;
            }
            let (list, rest) = match l.as_bytes().first() {
                Some(b'+') => (&mut plus, &l[1..]),
                Some(b'-') => (&mut minus, &l[1..]),
                _ => continue,
            };
            let Some((id, text)) = entry(rest) else {
                continue;
            };
            if !order.contains(&id) {
                order.push(id.clone());
            }
            list.retain(|(i, _)| *i != id);
            list.push((id, text));
        }
        let base = Path::new(f)
            .file_stem()
            .map_or_else(String::new, |s| s.to_string_lossy().into_owned());
        for id in order {
            let p = plus.iter().find(|(i, _)| *i == id);
            let m = minus.iter().find(|(i, _)| *i == id);
            let (kind, text, rev) = match (p, m) {
                (Some(p), Some(_)) => ("changed", p.1.clone(), format!("origin/{branch}")),
                (Some(p), None) => ("new", p.1.clone(), format!("origin/{branch}")),
                (None, _) => (
                    "dropped",
                    "- removed on the branch".to_owned(),
                    main.to_owned(),
                ),
            };
            let file = git::out(root, &["show", &format!("{rev}:{f}")]).unwrap_or_default();
            let needle = format!("**{id}**");
            let mut heading = "";
            for l in file.lines() {
                if l.starts_with("## ") {
                    heading = l;
                }
                if l.contains(&needle) {
                    break;
                }
            }
            out.push(format!("\n## {id} — {kind} · {base} `{heading}`\n{text}"));
        }
    }
    out
}

/// The run's slug from its branch `<prefix>/<slug>`; one that is no
/// [`board::valid_slug`] is refused.
fn slug_of(branch: &str) -> Result<String, Refused> {
    let slug = branch.split_once('/').map_or(branch, |(_, s)| s);
    if !board::valid_slug(slug) {
        return Err(Refused::BadBranch(branch.to_owned()));
    }
    Ok(slug.to_owned())
}

/// Every stage is `s<n>`, named once, and blocks only on `s<n>` stages;
/// checked before anything is written.
fn check_stages(st: &[Stage]) -> Result<(), Refused> {
    let sid = |d: &str| {
        d.strip_prefix('s')
            .is_some_and(|n| n.parse::<u32>().is_ok())
    };
    let mut seen = HashSet::new();
    for s in st {
        if !sid(&s.sid) || !seen.insert(s.sid.as_str()) {
            return Err(Refused::BadStage(s.sid.clone()));
        }
        if let Some(d) = s.blocked_by.iter().find(|d| !sid(d)) {
            return Err(Refused::BadStage(d.clone()));
        }
    }
    Ok(())
}

/// Recreates the board of the run on `pr`'s branch, `issue` its tracking
/// issue. Returns the slug, the stage count and the next stage.
pub fn rebuild(
    db: &mut Board,
    layout: &Layout,
    main: &str,
    issue_no: &str,
    pr_no: &str,
    source: &dyn Source,
    now: &str,
) -> Result<(String, usize, Option<String>), Error> {
    let conn = db.conn_mut();
    let pr = source.pr().ok_or(Refused::NoPr)?;
    let st = stages(&pr.body);
    if !pr.body.lines().any(|l| l == "## Stages") {
        return Err(Refused::NoStages.into());
    }
    let branch = source.branch().ok_or(Refused::NoBranch)?;
    let slug = slug_of(&branch)?;
    let exists = board::state(conn, &slug)?.is_some()
        || !board::stages(conn, &slug)?.is_empty()
        || layout.run_dir(&slug).exists();
    if exists {
        return Err(Refused::BoardExists.into());
    }
    let root = &layout.root;
    if !git::ok(root, &["fetch", "-q", "origin", &branch]) {
        return Err(Refused::NoBranch.into());
    }
    let issue = source.issue().ok_or(Refused::NoIssue)?;
    if st.is_empty() {
        return Err(Refused::NoStages.into());
    }
    check_stages(&st)?;
    for s in st.iter().filter(|s| s.done) {
        let sha = s.sha.clone().ok_or_else(|| Refused::NoSha(s.sid.clone()))?;
        if !git::ok(root, &["cat-file", "-e", &format!("{sha}^{{commit}}")]) {
            return Err(Refused::Sha(sha).into());
        }
    }
    super::ensure_worktree(
        layout,
        &layout.worktree(&slug),
        &branch,
        &format!("origin/{branch}"),
    )
    .map_err(Refused::Worktree)?;

    let lane = pr
        .body
        .lines()
        .find(|l| !l.is_empty())
        .and_then(|l| l.split(" · ").find_map(|w| w.strip_prefix("lane ")))
        .map_or("", |l| if l == "light" { "fast" } else { l })
        .to_owned();
    let today = now.get(..10).unwrap_or(now).to_owned();
    let mut fields = vec![("branch".to_owned(), branch.clone())];
    if !lane.is_empty() {
        fields.push(("lane".to_owned(), lane));
    }
    for (k, v) in [
        ("issue", issue_no),
        ("pr", pr_no),
        ("gate1", &today),
        ("arch", &today),
        ("gate2", &today),
    ] {
        fields.push((k.to_owned(), v.to_owned()));
    }
    let mut cards = vec![NewCard {
        id: slug.clone(),
        state: State::InProgress,
        fields,
        log: vec![
            (
                "rebuild".to_owned(),
                vec!["reason=board-missing".to_owned()],
            ),
            ("pr=draft".to_owned(), Vec::new()),
        ],
    }];
    let mut next = None;
    for s in &st {
        let deps: Vec<String> = s.blocked_by.iter().map(|d| format!("{slug}.{d}")).collect();
        cards.push(NewCard {
            id: format!("{slug}.{}", s.sid),
            state: if s.done { State::Done } else { State::Open },
            fields: vec![
                ("title".to_owned(), s.title.clone()),
                ("blocked-by".to_owned(), format!("[{}]", deps.join(", "))),
                ("files".to_owned(), "[]".to_owned()),
                ("branch".to_owned(), branch.clone()),
            ],
            log: if s.done {
                vec![(
                    format!("commit={}", s.sha.as_deref().unwrap_or("")),
                    Vec::new(),
                )]
            } else {
                Vec::new()
            },
        });
        if !s.done && next.is_none() {
            next = Some(s.sid.clone());
        }
    }
    board::create_all(conn, &cards, now).map_err(Error::Board)?;
    artifacts(layout, &slug, &issue, &pr, &st, main, &branch)?;
    Ok((slug, st.len(), next))
}

fn artifacts(
    layout: &Layout,
    slug: &str,
    issue: &Item,
    pr: &Item,
    st: &[Stage],
    main: &str,
    branch: &str,
) -> std::io::Result<()> {
    let dir = layout.run_dir(slug);
    std::fs::create_dir_all(&dir)?;
    let goal = trim_blank_end(section(&issue.body, "Goal"));
    let mut text = format!("# {}\n\n## Goal\n{}\n", issue.title(), goal.join("\n"));
    if issue.body.lines().any(|l| l == "## Out of scope") {
        let oos = trim_blank_end(section(&issue.body, "Out of scope"));
        let _ = write!(text, "\n## Out of scope\n{}\n", oos.join("\n"));
    }
    std::fs::write(dir.join("issue.md"), text)?;

    let first_goal = goal.iter().find(|l| !l.is_empty()).copied().unwrap_or("");
    let mut text = format!("# {slug} — spec diff (rev 1)\n\n## Summary\nGoal: {first_goal}\n");
    for l in section(&issue.body, "Scenarios") {
        if let Some(rest) = l.strip_prefix("- **")
            && let Some((id, title)) = rest.split_once("** — ")
        {
            let _ = write!(text, "\n## {id} — {title}\n");
        } else if let Some(step) = l.strip_prefix("  - ") {
            let _ = writeln!(text, "{step}");
        }
    }
    for e in spec_entries(&layout.root, main, branch) {
        let _ = writeln!(text, "{e}");
    }
    std::fs::write(dir.join("spec-diff.md"), text)?;

    let arch = section(&pr.body, "Architecture");
    if arch.iter().any(|l| !l.is_empty()) {
        let decide = arch.iter().find(|l| !l.is_empty()).copied().unwrap_or("");
        let mut text = format!("# {slug} — architecture summary (rebuilt)\n\nDecide: {decide}\n");
        if let Some(i) = arch.iter().position(|l| l.starts_with("Public surface:")) {
            text.push('\n');
            text.push_str(arch[i]);
            text.push('\n');
            for l in arch[i + 1..].iter().take_while(|l| l.starts_with("- ")) {
                let _ = writeln!(text, "{l}");
            }
        }
        std::fs::write(dir.join("architecture.summary.md"), text)?;
        let table: Vec<&str> = arch
            .iter()
            .copied()
            .filter(|l| l.starts_with('|'))
            .collect();
        if !table.is_empty() {
            std::fs::write(
                dir.join("architecture.md"),
                format!(
                    "# {slug} — architecture (rebuilt)\n\n## Components\n\n{}\n",
                    table.join("\n")
                ),
            )?;
        }
    }

    let mut text = format!(
        "# {slug} — plan summary (rebuilt)\n\n| Stage | What | Pins | Files | Stop |\n|---|---|---|---|---|\n"
    );
    for s in st {
        let _ = writeln!(text, "| {} | {} | — | — | — |", s.sid, s.title);
    }
    if st.iter().any(|s| s.verification.is_some()) {
        text.push_str("\nVerification beyond unit tests:\n");
        for s in st {
            if let Some((what, subs)) = &s.verification {
                let _ = writeln!(text, "- {} — {what}", s.sid);
                for sub in subs {
                    let _ = writeln!(text, "  - {sub}");
                }
            }
        }
    }
    std::fs::write(dir.join("plan.summary.md"), text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ut_stage_lines() {
        let pr = "## Stages\n- [x] Land spec (abc1234) <!-- s0 -->\n- [ ] Retry — next <!-- s1 blocked-by s0 -->\n  - beyond unit tests: demo\n    - see it\n\n## Review here\n";
        let s = stages(pr);
        assert_eq!(s.len(), 2);
        assert_eq!(
            (s[0].done, s[0].sha.as_deref(), s[0].title.as_str()),
            (true, Some("abc1234"), "Land spec")
        );
        assert_eq!(s[1].blocked_by, ["s0"]);
        assert_eq!(
            s[1].verification,
            Some(("demo".into(), vec!["see it".into()]))
        );
    }

    #[test]
    fn ut_entries() {
        assert_eq!(
            entry("**MB-R-1** — text"),
            Some(("MB-R-1".into(), "text".into()))
        );
        assert_eq!(
            entry("| **MB-E-1** | Slow | skipped |"),
            Some(("MB-E-1".into(), "Slow — skipped".into()))
        );
        assert_eq!(entry("plain"), None);
    }
}
