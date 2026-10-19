use std::io::{self, Write};
use std::path::PathBuf;

use clap::{Args, Subcommand, ValueEnum};
use ferrule_core::card::board;
use ferrule_core::config::Config;
use ferrule_core::pr::publish;
use ferrule_core::project::{self, Layout};
use ferrule_core::remote::{self, Item};
use ferrule_core::run::archive::{self, Error as ArchiveError};
use ferrule_core::run::gate::{self, Gate};
use ferrule_core::run::gauntlet::{self, Outcome};
use ferrule_core::run::next::{self, At, Status};
use ferrule_core::run::rebuild;
use ferrule_core::run::{self as core_run, Error};
use ferrule_core::store::DIR;

use super::card::{sql, store};
use super::exit;
use crate::cli::stage_id;

/// Shown after `ferrule run next --help`.
const NEXT_AFTER_LONG_HELP: &str = "\
Statuses each position takes:
  gate2    ready, findings, clean, question, arch-gap, split, behavior-change,
           new, reuse (with issue=<n>)
  replan   ready, findings, clean, question, arch-gap, split, behavior-change
  gate3    findings (with count=<n>), clean
  gate4    ready, question
  ci       ci=<exit code of gh pr checks>

Example:
  ferrule run next <slug> gate2 status=ready file=plan.md";

#[derive(Debug, Subcommand)]
pub enum RunCommand {
    /// Start a run: its parent card and directory.
    ///
    /// With `--issue`, also the issue's request, its brief and, for a
    /// slice, its epic.
    Init {
        /// The run's slug.
        slug: String,
        /// The issue the user named: a GitHub number or URL, or a Jira key.
        #[arg(long)]
        issue: Option<String>,
    },
    /// Record an approved gate.
    ///
    /// `gate2` also files the issue, adds the worktree, moves the parent
    /// card to inprogress and creates the stage cards.
    Approve(ApproveArgs),
    /// Log an agent's outcome and print the next step.
    ///
    /// Prints `next=`, then `run:` and `do:` lines and a `guide:` line
    /// naming the topic to read next.
    ///
    /// Reads `status=`, `count=`, `issue=` and `ci=` from the status line and
    /// ignores the rest. Prints `run-next=unknown` (exit 2) for a status the
    /// position does not take.
    #[command(
        after_help = "Example:\n  ferrule run next <slug> gate2 status=ready file=plan.md",
        after_long_help = NEXT_AFTER_LONG_HELP
    )]
    Next {
        /// The run's slug.
        slug: String,
        /// Where the run is: the step whose agent wrote the status line.
        #[arg(value_enum)]
        position: NextAt,
        /// The agent's status line, as one argument or several.
        #[arg(
            required = true,
            allow_hyphen_values = true,
            value_name = "STATUS_LINE"
        )]
        status: Vec<String>,
    },
    /// Run the gauntlet in the run's worktree.
    ///
    /// The steps are `[[gauntlet.step]]` of `ferrule.toml`. Prints one
    /// status line; the output goes to the run's gauntlet.log.
    Gauntlet {
        /// The run's slug.
        slug: String,
        /// Run even when this tree passed before.
        #[arg(long)]
        force: bool,
    },
    /// Verify a stage's reported commit before anything is pushed.
    ///
    /// The commit exists in the worktree, the run's branch holds it, and
    /// it is the stage card's last `commit=`.
    ShaCheck {
        /// The run's slug.
        slug: String,
        /// The stage, `s<n>`.
        #[arg(value_parser = stage_id, value_name = "STAGE")]
        stage: u32,
        /// The commit the implementer reported.
        sha: String,
    },
    /// Decide whether a green full-lane stage stops for approval.
    ///
    /// Logs the outcome on the stage card.
    StageStop {
        /// The run's slug.
        slug: String,
        /// The stage, `s<n>`.
        #[arg(value_parser = stage_id, value_name = "STAGE")]
        stage: u32,
        /// Unresolved PR review threads.
        #[arg(long, default_value_t = 0)]
        threads: u32,
    },
    /// Start or approve a re-plan.
    Replan {
        /// The run's slug.
        slug: String,
        /// Which half of the re-plan.
        step: ReplanStep,
        /// With `approve`: stages that stop for approval beyond the plan's,
        /// e.g. `s1,s3`.
        #[arg(long)]
        stops: Option<String>,
    },
    /// Wait for the PR's checks and print the next step.
    ///
    /// Logs `ci=`; on red, also prints the failed run's error lines.
    Ci {
        /// The run's slug.
        slug: String,
    },
    /// Reconcile a run's cards with git and the host before resuming.
    ///
    /// Prints one row per check; run it before an interrupted run goes on.
    Reconcile {
        /// The run's slug.
        slug: String,
        /// Check git only: skip the issue, PR and PR-body rows.
        #[arg(long)]
        local: bool,
    },
    /// Recreate a lost board from the issue, its draft PR and git.
    ///
    /// Also adds the worktree back.
    Rebuild {
        /// The tracking issue: a GitHub number or URL, or a Jira key.
        issue: String,
        /// The run's draft PR: a number or URL.
        pr: String,
    },
    /// Clean up a finished run.
    ///
    /// Removes the worktree and the local branch and archives the cards and
    /// files. Refuses while the worktree holds unpushed work or CI is not
    /// green.
    Cleanup {
        /// The run's slug.
        slug: String,
    },
    /// Record and print a run's metrics line.
    ///
    /// Needs the run's CI green. Without a slug, prints every line recorded.
    Metrics {
        /// The run's slug; without one, every line recorded.
        slug: Option<String>,
    },
    /// Write an archived run back out as files.
    Export {
        /// The run's slug.
        slug: String,
        /// The archive id `run cleanup` printed [default: the latest].
        #[arg(long)]
        archive: Option<i64>,
        /// Target directory [default: .ferrule/export/<slug>-<archive>].
        #[arg(long)]
        to: Option<PathBuf>,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum GateName {
    /// The brief.
    Gate1,
    /// The architecture.
    Arch,
    /// An epic's slices.
    Epic,
    /// The plan and the tracking issue.
    Gate2,
    /// A stage stop.
    Stage,
    /// A re-plan.
    Replan,
    /// A meaningful spec change.
    Delta,
    /// The branch review.
    Gate3,
    /// The pull request.
    Gate4,
}

#[derive(Debug, Args)]
pub struct ApproveArgs {
    /// The run's slug.
    pub slug: String,
    /// The gate the user approved.
    pub gate: GateName,
    /// The lane the user settled on, recorded on the card.
    #[arg(long, value_parser = ["fast", "full"])]
    pub lane: Option<String>,
    /// gate1: the run is an epic.
    #[arg(long)]
    pub epic: bool,
    /// gate2: `new`, or the number of an issue the run reuses.
    #[arg(long)]
    pub issue: Option<String>,
    /// gate2: the branch [default: feat/<slug>].
    #[arg(long)]
    pub branch: Option<String>,
    /// gate2: stages that stop for approval beyond the plan's, e.g. `s1,s3`.
    #[arg(long)]
    pub stops: Option<String>,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum NextAt {
    /// Planner, plan reviewer, the spec author's issue answer.
    Gate2,
    /// A re-plan's planner and plan reviewer.
    Replan,
    /// The branch review.
    Gate3,
    /// The spec author's PR body.
    Gate4,
    /// `ci=<exit code of gh pr checks>`.
    Ci,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ReplanStep {
    /// Log `replan from=` and snapshot the plan.
    Start,
    /// Swap the stage cards from `from` on for the new plan's.
    Approve,
}

fn layout() -> Layout {
    Layout::new(project::main_checkout())
}

pub fn run(command: &RunCommand, out: &mut impl Write) -> io::Result<u8> {
    match command {
        RunCommand::ShaCheck { slug, stage, sha } => sha_check(slug, *stage, sha, out),
        RunCommand::Reconcile { slug, local } => reconcile(slug, *local, out),
        RunCommand::Init { slug, issue } => init(slug, issue.as_deref(), out),
        RunCommand::Approve(args) => approve(args, out),
        RunCommand::Replan { slug, step, stops } => replan(slug, *step, stops.as_deref(), out),
        RunCommand::Ci { slug } => ci(slug, out),
        RunCommand::Gauntlet { slug, force } => gauntlet(slug, *force, out),
        RunCommand::Next {
            slug,
            position,
            status,
        } => next_step(slug, *position, status, out),
        RunCommand::Metrics { slug } => metrics(slug.as_deref(), out),
        RunCommand::Cleanup { slug } => cleanup(slug, out),
        RunCommand::Rebuild { issue, pr } => rebuild(issue, pr, out),
        RunCommand::Export { slug, archive, to } => export(slug, *archive, to.as_deref(), out),
        RunCommand::StageStop {
            slug,
            stage,
            threads,
        } => stage_stop(slug, *stage, *threads, out),
    }
}

fn sha_check(slug: &str, stage: u32, sha: &str, out: &mut impl Write) -> io::Result<u8> {
    let verdict = match store(false)? {
        Some(s) => {
            core_run::sha_check(&s.conn, &layout(), slug, &format!("s{stage}"), sha).map_err(sql)?
        }
        None => Err(core_run::Mismatch::Card),
    };
    match verdict {
        Ok(()) => {
            writeln!(out, "sha=ok")?;
            Ok(exit::OK)
        }
        Err(m) => {
            writeln!(out, "sha=mismatch reason={}", m.as_str())?;
            Ok(exit::REFUSED)
        }
    }
}

fn reconcile(slug: &str, local: bool, out: &mut impl Write) -> io::Result<u8> {
    let layout = layout();
    let mut store = store(false)?;
    let result = match &store {
        Some(s) => core_run::reconcile(&s.conn, &layout, slug),
        None => Err(Error::Card),
    };
    let mut r = match result {
        Ok(r) => r,
        Err(Error::Card) => {
            eprintln!("reconcile: no card for {slug}");
            return Ok(exit::USAGE);
        }
        Err(Error::Sql(e)) => return Err(sql(e)),
    };
    if !local && let Some(s) = store.as_mut() {
        let config = Config::load(&layout.root).map_err(sql)?;
        let now = board::now();
        let run = publish::Run {
            conn: &mut s.conn,
            layout: &layout,
            config: &config,
            root: &layout.root,
            slug,
            now: &now,
        };
        match run.reconcile_remote() {
            Ok(rows) => r.differ.extend(rows),
            Err(e) => r.differ.push(format!("host unreadable: {e:?}")),
        }
    }
    for d in &r.differ {
        writeln!(out, "differ row={d}")?;
    }
    let next = r.next.as_deref().unwrap_or("none");
    if r.differ.is_empty() {
        writeln!(out, "reconcile=clean next={next}")?;
        Ok(exit::OK)
    } else {
        writeln!(out, "reconcile=differ n={} next={next}", r.differ.len())?;
        Ok(exit::REFUSED)
    }
}

fn stage_stop(slug: &str, stage: u32, threads: u32, out: &mut impl Write) -> io::Result<u8> {
    let layout = layout();
    let config = Config::load(&layout.root).map_err(sql)?;
    let Some(mut s) = store(false)? else {
        writeln!(out, "stage-stop=card")?;
        return Ok(exit::REFUSED);
    };
    match core_run::stage_stop(
        &mut s.conn,
        &layout,
        &config,
        slug,
        stage,
        threads,
        &board::now(),
    ) {
        Ok(Some(stop)) => writeln!(out, "{stop}")?,
        Ok(None) => writeln!(out, "auto")?,
        Err(Error::Card) => {
            writeln!(out, "stage-stop=card")?;
            return Ok(exit::REFUSED);
        }
        Err(Error::Sql(e)) => return Err(sql(e)),
    }
    Ok(exit::OK)
}

fn metrics(slug: Option<&str>, out: &mut impl Write) -> io::Result<u8> {
    let Some(s) = store(false)? else {
        if let Some(slug) = slug {
            eprintln!("metrics: no card for {slug}");
            return Ok(exit::REFUSED);
        }
        return Ok(exit::OK);
    };
    let Some(slug) = slug else {
        for line in archive::metrics_log(&s.conn).map_err(sql)? {
            writeln!(out, "{line}")?;
        }
        return Ok(exit::OK);
    };
    match archive::metrics(&s.conn, slug, &board::now()) {
        Ok(line) => {
            writeln!(out, "{line}")?;
            Ok(exit::OK)
        }
        Err(ArchiveError::Card) => {
            eprintln!("metrics: no card for {slug}");
            Ok(exit::REFUSED)
        }
        Err(ArchiveError::Ci(r)) => {
            writeln!(out, "metrics=refused reason={}", r.as_str())?;
            Ok(exit::REFUSED)
        }
        Err(e) => Err(sql(format!("{e:?}"))),
    }
}

fn cleanup(slug: &str, out: &mut impl Write) -> io::Result<u8> {
    let layout = layout();
    let Some(mut s) = store(true)? else {
        unreachable!("store(true) creates the database");
    };
    let reason = match archive::cleanup(&mut s.conn, &layout, slug, &board::now()) {
        Ok(Some(id)) => {
            writeln!(out, "cleanup=done archive={id}")?;
            return Ok(exit::OK);
        }
        Ok(None) => {
            writeln!(out, "cleanup=done archive=none")?;
            return Ok(exit::OK);
        }
        Err(ArchiveError::Refused(r)) => r,
        Err(ArchiveError::Ci(r)) => r.as_str(),
        Err(ArchiveError::Card) => "card",
        Err(ArchiveError::Io(e)) => return Err(e),
        Err(ArchiveError::Sql(e)) => return Err(sql(e)),
    };
    writeln!(out, "cleanup=refused reason={reason}")?;
    Ok(exit::REFUSED)
}

fn export(
    slug: &str,
    id: Option<i64>,
    to: Option<&std::path::Path>,
    out: &mut impl Write,
) -> io::Result<u8> {
    let layout = layout();
    let Some(s) = store(false)? else {
        writeln!(out, "export=none")?;
        return Ok(exit::REFUSED);
    };
    let id = match id {
        Some(id) => Some(id),
        None => archive::archives(&s.conn, slug)
            .map_err(sql)?
            .last()
            .map(|(id, _)| *id),
    };
    let Some(id) = id else {
        writeln!(out, "export=none")?;
        return Ok(exit::REFUSED);
    };
    let dest = to.map_or_else(
        || {
            layout
                .root
                .join(DIR)
                .join("export")
                .join(format!("{slug}-{id}"))
        },
        std::path::Path::to_path_buf,
    );
    match archive::export(&s.conn, slug, Some(id), &dest) {
        Ok(Some((id, path))) => {
            writeln!(out, "export=done archive={id} path={}", path.display())?;
            Ok(exit::OK)
        }
        Ok(None) => {
            writeln!(out, "export=none")?;
            Ok(exit::REFUSED)
        }
        Err(ArchiveError::Io(e)) => Err(e),
        Err(e) => Err(sql(format!("{e:?}"))),
    }
}

fn gauntlet(slug: &str, force: bool, out: &mut impl Write) -> io::Result<u8> {
    let layout = layout();
    let config = Config::load(&layout.root).map_err(sql)?;
    let Some(s) = store(true)? else {
        unreachable!("store(true) creates the database");
    };
    let wt = board::field(&s.conn, slug, "worktree")
        .map_err(sql)?
        .map_or_else(|| layout.worktree(slug), |w| layout.resolve(&w));
    let log = layout.run_dir(slug).join("gauntlet.log");
    let outcome = gauntlet::run(
        &s.conn,
        slug,
        &wt,
        &log,
        &config.gauntlet.step,
        force,
        &board::now(),
    )
    .map_err(sql)?;
    writeln!(out, "{outcome}")?;
    Ok(if matches!(outcome, Outcome::Pass { .. }) {
        exit::OK
    } else {
        exit::REFUSED
    })
}

fn next_step(slug: &str, at: NextAt, status: &[String], out: &mut impl Write) -> io::Result<u8> {
    let at = match at {
        NextAt::Gate2 => At::Gate2,
        NextAt::Replan => At::Replan,
        NextAt::Gate3 => At::Gate3,
        NextAt::Gate4 => At::Gate4,
        NextAt::Ci => At::Ci,
    };
    let status = Status::parse(status);
    let Some(mut s) = store(false)? else {
        writeln!(out, "run-next=card")?;
        return Ok(exit::REFUSED);
    };
    match next::next(&mut s.conn, &layout(), slug, at, &status, &board::now()) {
        Ok(lines) => {
            for l in lines {
                writeln!(out, "{l}")?;
            }
            Ok(exit::OK)
        }
        Err(next::Error::Usage) => {
            eprintln!(
                "usage: ferrule run next <slug> {} status=<word>…",
                at.as_str()
            );
            Ok(exit::USAGE)
        }
        Err(next::Error::Card) => {
            writeln!(out, "run-next=card")?;
            Ok(exit::REFUSED)
        }
        Err(next::Error::Unknown(word)) => {
            writeln!(out, "run-next=unknown at={} status={word}", at.as_str())?;
            Ok(exit::USAGE)
        }
        Err(next::Error::Sql(e)) => Err(sql(e)),
    }
}

/// The remote host behind `run rebuild`.
struct Remote {
    config: Config,
    pr: String,
    issue: String,
    root: std::path::PathBuf,
}

impl rebuild::Source for Remote {
    fn pr(&self) -> Option<Item> {
        remote::pr(self.config.backend.ci, &self.pr, &self.root).ok()
    }

    fn branch(&self) -> Option<String> {
        remote::pr_branch(self.config.backend.ci, &self.pr, &self.root).ok()
    }

    fn issue(&self) -> Option<Item> {
        remote::issue(self.config.backend.tracker, &self.issue, &self.root).ok()
    }
}

fn rebuild(issue: &str, pr: &str, out: &mut impl Write) -> io::Result<u8> {
    let layout = Layout::new(project::main_checkout());
    let config = Config::load(&layout.root).map_err(sql)?;
    let main = config.main_branch.clone();
    let source = Remote {
        config,
        pr: pr.to_owned(),
        issue: issue.to_owned(),
        root: layout.root.clone(),
    };
    let Some(mut s) = store(true)? else {
        return Ok(exit::REFUSED);
    };
    match rebuild::rebuild(
        &mut s.conn,
        &layout,
        &main,
        issue,
        pr,
        &source,
        &board::now(),
    ) {
        Ok((slug, n, next)) => {
            writeln!(
                out,
                "rebuild=done slug={slug} stages={n} next={}",
                next.as_deref().unwrap_or("none")
            )?;
            Ok(exit::OK)
        }
        Err(rebuild::Error::Refused(r)) => {
            writeln!(out, "{r}")?;
            Ok(exit::REFUSED)
        }
        Err(rebuild::Error::Sql(e)) => Err(sql(e)),
        Err(rebuild::Error::Board(e)) => Err(sql(format!("{e:?}"))),
        Err(rebuild::Error::Io(e)) => Err(e),
    }
}

fn gate_failed(e: gate::Error, verb: &str, out: &mut impl Write) -> io::Result<u8> {
    match e {
        gate::Error::Refused(reason) => {
            writeln!(out, "{verb}=refused reason={reason}")?;
            Ok(exit::REFUSED)
        }
        gate::Error::Publish(e) => super::issue::post_failed(e, out),
        gate::Error::Remote(e) => {
            eprintln!("{verb}: {e}");
            Ok(exit::REFUSED)
        }
        gate::Error::Board(e) => Err(sql(format!("{e:?}"))),
        gate::Error::Io(e) => Err(e),
        gate::Error::Sql(e) => Err(sql(e)),
    }
}

fn init(slug: &str, issue: Option<&str>, out: &mut impl Write) -> io::Result<u8> {
    let layout = layout();
    let config = Config::load(&layout.root).map_err(sql)?;
    let Some(mut s) = store(true)? else {
        return Ok(exit::REFUSED);
    };
    match gate::init(&mut s.conn, &layout, &config, slug, issue, &board::now()) {
        Ok(epic) => {
            write!(out, "init=done slug={slug}")?;
            if let Some(e) = epic {
                write!(out, " kind=slice epic={e}")?;
            }
            writeln!(out)?;
            Ok(exit::OK)
        }
        Err(e) => gate_failed(e, "init", out),
    }
}

fn approve(args: &ApproveArgs, out: &mut impl Write) -> io::Result<u8> {
    let (g, gate_name) = match args.gate {
        GateName::Gate1 => (Gate::Gate1, "gate1"),
        GateName::Arch => (Gate::Arch, "arch"),
        GateName::Epic => (Gate::Epic, "epic"),
        GateName::Gate2 => (Gate::Gate2, "gate2"),
        GateName::Stage => (Gate::Other, "stage"),
        GateName::Replan => (Gate::Other, "replan"),
        GateName::Delta => (Gate::Other, "delta"),
        GateName::Gate3 => (Gate::Other, "gate3"),
        GateName::Gate4 => (Gate::Other, "gate4"),
    };
    let slug = args.slug.as_str();
    let mut pairs = Vec::new();
    let mut pair = |k: &str, v: Option<&String>| {
        if let Some(v) = v {
            pairs.push((k.to_owned(), v.clone()));
        }
    };
    pair("lane", args.lane.as_ref());
    pair("issue", args.issue.as_ref());
    pair("branch", args.branch.as_ref());
    pair("stops", args.stops.as_ref());
    if args.epic {
        pairs.push(("kind".to_owned(), "epic".to_owned()));
    }
    let layout = layout();
    let config = Config::load(&layout.root).map_err(sql)?;
    let Some(mut s) = store(false)? else {
        writeln!(out, "approve=refused reason=card")?;
        return Ok(exit::REFUSED);
    };
    match gate::approve(
        &mut s.conn,
        &layout,
        &config,
        g,
        slug,
        &pairs,
        &board::now(),
    ) {
        Ok(a) => {
            write!(out, "approve=done gate={gate_name}")?;
            if let Some(i) = a.issue {
                write!(out, " issue={i}")?;
            }
            if let Some(w) = &a.worktree {
                write!(out, " worktree={w}")?;
            }
            if let Some(n) = a.stages {
                write!(out, " stages={n}")?;
            }
            if !a.record.is_empty() {
                write!(out, " {}", a.record)?;
            }
            writeln!(out)?;
            Ok(exit::OK)
        }
        Err(e) => gate_failed(e, "approve", out),
    }
}

fn replan(
    slug: &str,
    step: ReplanStep,
    stops: Option<&str>,
    out: &mut impl Write,
) -> io::Result<u8> {
    let layout = layout();
    let Some(mut s) = store(false)? else {
        writeln!(out, "replan=refused reason=card")?;
        return Ok(exit::REFUSED);
    };
    let now = board::now();
    let result = match step {
        ReplanStep::Start => gate::replan_start(&mut s.conn, &layout, slug, &now)
            .map(|from| format!("replan=started from=s{from}")),
        ReplanStep::Approve => {
            let stops: Vec<u32> = stops
                .unwrap_or("")
                .split(',')
                .filter_map(|x| x.trim().strip_prefix('s')?.parse().ok())
                .collect();
            gate::replan_approve(&mut s.conn, &layout, slug, &stops, &now)
                .map(|(from, n)| format!("replan=approved from=s{from} stages={n}"))
        }
    };
    match result {
        Ok(line) => {
            writeln!(out, "{line}")?;
            if matches!(step, ReplanStep::Approve) {
                super::card::sync_draft(&mut s, slug, out)?;
            }
            Ok(exit::OK)
        }
        Err(e) => gate_failed(e, "replan", out),
    }
}

fn ci(slug: &str, out: &mut impl Write) -> io::Result<u8> {
    let layout = layout();
    let config = Config::load(&layout.root).map_err(sql)?;
    let Some(mut s) = store(false)? else {
        writeln!(out, "ci=refused reason=card")?;
        return Ok(exit::REFUSED);
    };
    let now = board::now();
    let waited = publish::Run {
        conn: &mut s.conn,
        layout: &layout,
        config: &config,
        root: &layout.root,
        slug,
        now: &now,
    }
    .wait_ci();
    let (code, failed) = match waited {
        Ok(r) => r,
        Err(e) => return super::issue::post_failed(e, out),
    };
    write!(out, "ci={code}")?;
    if !failed.is_empty() {
        write!(out, " failed={}", failed.join(","))?;
    }
    writeln!(out)?;
    if code != 0 {
        let branch = board::field(&s.conn, slug, "branch")
            .map_err(sql)?
            .unwrap_or_default();
        if let Ok(lines) = ferrule_core::remote::failed(
            config.backend.ci,
            &branch,
            ferrule_core::remote::Query::default(),
            &layout.root,
        ) {
            for l in lines {
                writeln!(out, "{l}")?;
            }
        }
    }
    let status = Status::parse(&[format!("ci={code}")]);
    match next::next(&mut s.conn, &layout, slug, At::Ci, &status, &now) {
        Ok(lines) => {
            for l in lines {
                writeln!(out, "{l}")?;
            }
            Ok(exit::OK)
        }
        Err(e) => Err(sql(format!("{e:?}"))),
    }
}
