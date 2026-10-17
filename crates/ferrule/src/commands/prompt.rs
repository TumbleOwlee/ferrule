use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use ferrule_core::config::Config;
use ferrule_core::project::{self, Layout};
use ferrule_core::prompt_render::{self, Kind, Viewed};
use ferrule_core::{approval, board, prompt_lines};

use super::card::{sql, store};
use super::exit;
use crate::cli::PromptCommand;

pub fn run(command: &PromptCommand, out: &mut impl Write) -> io::Result<u8> {
    match command {
        PromptCommand::Lines { files, seen } => lines(files, *seen, out),
        PromptCommand::Show { files, no_lines } => show(files, !*no_lines, out),
        PromptCommand::Render { kind, slug, args } => render(kind, slug, args, out),
    }
}

fn lines(files: &[PathBuf], seen: usize, out: &mut impl Write) -> io::Result<u8> {
    let paths: Vec<&Path> = files.iter().map(PathBuf::as_path).collect();
    let (lines, unreadable) = prompt_lines::lines(&paths, seen);
    for l in lines {
        writeln!(out, "{l}")?;
    }
    for u in &unreadable {
        eprintln!("prompt lines: not readable: {u}");
    }
    Ok(if unreadable.is_empty() {
        exit::OK
    } else {
        exit::REFUSED
    })
}

/// How a file was shown, or why not.
pub enum Shown {
    Viewer(String),
    Stdout,
    Failed,
}

/// Runs the viewer on `path` with its output kept out of the caller's.
fn view(viewer: &[String], path: &Path) -> Shown {
    let Some((program, args)) = viewer.split_first() else {
        return Shown::Stdout;
    };
    let ok = Command::new(program)
        .args(args)
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    if ok {
        Shown::Viewer(program.clone())
    } else {
        Shown::Failed
    }
}

/// A file shown, how, and the run whose approval marker it joined.
pub type ShownFile = (PathBuf, Shown, Option<String>);

/// Validates, records and opens `files`; the shown line per file. Refuses
/// everything when one file is unreadable (exit 1) or a summary is over
/// its cap (exit 3).
pub fn open(files: &[PathBuf], err: &mut impl Write) -> io::Result<Result<Vec<ShownFile>, u8>> {
    let layout = Layout::new(project::main_checkout());
    let config = Config::load(&layout.root).map_err(sql)?;
    let mut abs = Vec::new();
    for f in files {
        let Ok(text) = std::fs::read_to_string(f) else {
            writeln!(err, "prompt show: not readable: {}", f.display())?;
            return Ok(Err(exit::REFUSED));
        };
        if let Some((lines, bytes)) = approval::over_cap(f, &text, &config.thresholds) {
            writeln!(
                err,
                "prompt show: refused: {} is {lines} lines / {bytes} bytes, cap is {} lines / {} bytes; have the agent that wrote it trim it, never show the full file instead",
                f.display(),
                config.thresholds.summary_max_lines,
                config.thresholds.summary_max_bytes
            )?;
            return Ok(Err(exit::CONTENT));
        }
        abs.push(f.canonicalize()?);
    }
    let runs = match store(true)? {
        Some(mut s) => approval::record(&mut s.conn, &layout, &abs, &board::now()).map_err(sql)?,
        None => vec![None; abs.len()],
    };
    let viewer = config.viewer.command.unwrap_or_default();
    Ok(Ok(abs
        .into_iter()
        .zip(runs)
        .map(|(path, run)| {
            let shown = view(&viewer, &path);
            (path, shown, run)
        })
        .collect()))
}

fn show(files: &[PathBuf], with_lines: bool, out: &mut impl Write) -> io::Result<u8> {
    let shown = match open(files, &mut io::stderr())? {
        Ok(s) => s,
        Err(code) => return Ok(code),
    };
    let mut code = exit::OK;
    for (path, how, run) in shown {
        let marker = run
            .map(|r| format!(" (approval marker: {r})"))
            .unwrap_or_default();
        match how {
            Shown::Viewer(v) => writeln!(
                out,
                "prompt show: opened with {v}: {}{marker}",
                path.display()
            )?,
            Shown::Failed => {
                writeln!(
                    out,
                    "prompt show: viewer failed: {}{marker}",
                    path.display()
                )?;
                code = exit::REFUSED;
            }
            Shown::Stdout => {
                writeln!(out, "prompt show: stdout: {}{marker}", path.display())?;
                out.write_all(&std::fs::read(&path)?)?;
                continue;
            }
        }
        if with_lines {
            let (lines, _) = prompt_lines::lines(&[path.as_path()], 0);
            for l in lines {
                writeln!(out, "{l}")?;
            }
        }
    }
    Ok(code)
}

fn render(kind: &str, slug: &str, args: &[String], out: &mut impl Write) -> io::Result<u8> {
    let usage = || {
        eprintln!(
            "usage: ferrule prompt render <gate1|arch|epic|gate2|stage|replan|gate3|gate4|question|blocked|delta|auto|gate2-notice|replan-notice|approved> <slug> [k=v ...]"
        );
        Ok(exit::USAGE)
    };
    let Some(kind) = Kind::parse(kind) else {
        return usage();
    };
    let mut pairs = Vec::new();
    for a in args {
        let Some((k, v)) = a.split_once('=') else {
            return usage();
        };
        pairs.push((k.to_owned(), v.to_owned()));
    }
    let Some(mut s) = store(false)? else {
        eprintln!("prompt render: no card for {slug}");
        return Ok(exit::USAGE);
    };
    let layout = Layout::new(project::main_checkout());
    let mut view = |files: &[PathBuf]| -> Result<Viewed, u8> {
        let shown = match open(files, &mut io::stderr()) {
            Ok(Ok(s)) => s,
            Ok(Err(code)) => return Err(code),
            Err(_) => return Ok(Viewed::Failed),
        };
        if shown.iter().any(|(_, how, _)| matches!(how, Shown::Failed)) {
            Ok(Viewed::Failed)
        } else if shown
            .iter()
            .all(|(_, how, _)| matches!(how, Shown::Viewer(_)))
        {
            Ok(Viewed::Opened)
        } else {
            Ok(Viewed::Unavailable(
                shown.into_iter().map(|(p, _, _)| p).collect(),
            ))
        }
    };
    match prompt_render::render(
        &mut s.conn,
        &layout,
        kind,
        slug,
        &pairs,
        &board::now(),
        &mut view,
    ) {
        Ok(lines) => {
            for l in lines {
                writeln!(out, "{l}")?;
            }
            Ok(exit::OK)
        }
        Err(prompt_render::Error::Usage) => usage(),
        Err(prompt_render::Error::Card) => {
            eprintln!("prompt render: no card for {slug}");
            Ok(exit::USAGE)
        }
        Err(prompt_render::Error::Show(code)) => Ok(code),
        Err(prompt_render::Error::Sql(e)) => Err(sql(e)),
    }
}
