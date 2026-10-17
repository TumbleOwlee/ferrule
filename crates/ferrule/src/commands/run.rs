use std::io::{self, Write};

use ferrule_core::archive::{self, Error as ArchiveError};
use ferrule_core::board;
use ferrule_core::config::Config;
use ferrule_core::gate::{self, Gate};
use ferrule_core::gauntlet::{self, Outcome};
use ferrule_core::next::{self, At, Status};
use ferrule_core::project::{self, Layout};
use ferrule_core::publish;
use ferrule_core::rebuild;
use ferrule_core::remote::{self, Item};
use ferrule_core::run::{self as core_run, Error};
use ferrule_core::store::DIR;

use super::card::{sql, store};
use super::exit;
use crate::cli::{ApproveArgs, GateName, NextAt, ReplanStep, RunCommand};

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
        RunCommand::Next { slug, at, status } => next_step(slug, *at, status, out),
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
        if let Ok(lines) = ferrule_core::remote::failed(config.backend.ci, &branch, &layout.root) {
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
