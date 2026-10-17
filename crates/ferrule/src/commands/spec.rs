use std::collections::{BTreeSet, HashSet};
use std::io::{self, Write};
use std::path::Path;
use std::process::Command;

use ferrule_core::markdown::{self, Match};
use ferrule_core::spec_diff::{self, Part, SpecDiff};
use ferrule_core::{board, project, spec};

use super::exit;
use crate::cli::{
    CollisionsArgs, IdArgs, RelatedArgs, RenderArgs, RenderPart, SectionArgs, SectionsArgs,
    SpecCommand,
};

/// Longest entry text `spec related` prints before cutting it with `…`.
const RELATED_TEXT_MAX: usize = 100;

pub fn run(command: &SpecCommand, out: &mut impl Write) -> io::Result<u8> {
    match command {
        SpecCommand::Section(args) => section(args, out),
        SpecCommand::Sections(args) => sections(args, out),
        SpecCommand::Id(args) => id(args, out),
        SpecCommand::Related(args) => related(args, out),
        SpecCommand::Collisions(args) => collisions(args, out),
        SpecCommand::Render(args) => render(args, out),
    }
}

fn read(path: &Path) -> Option<String> {
    std::fs::read_to_string(path)
        .inspect_err(|e| eprintln!("cannot read {}: {e}", path.display()))
        .ok()
}

fn section(args: &SectionArgs, out: &mut impl Write) -> io::Result<u8> {
    let Some(text) = read(&args.file) else {
        return Ok(exit::REFUSED);
    };
    let how = if args.prefix {
        Match::Prefix
    } else {
        Match::Exact
    };
    let mut missing = Vec::new();
    for want in &args.headings {
        let lines = markdown::section_lines(&text, want, how);
        if lines.is_empty() {
            missing.push(want.as_str());
        } else {
            // Trailing blank lines are dropped: a section ends at its last
            // line of content.
            writeln!(out, "{}", lines.join("\n").trim_end_matches('\n'))?;
        }
    }
    if missing.is_empty() {
        return Ok(exit::OK);
    }
    eprintln!("no section matched:\n{}", missing.join("\n"));
    Ok(exit::REFUSED)
}

fn sections(args: &SectionsArgs, out: &mut impl Write) -> io::Result<u8> {
    let mut code = exit::OK;
    for file in &args.files {
        if args.files.len() > 1 {
            writeln!(out, "== {} ==", file.display())?;
        }
        let Some(text) = read(file) else {
            code = exit::REFUSED;
            continue;
        };
        for heading in markdown::headings(&text) {
            writeln!(out, "{}", heading.line)?;
        }
    }
    Ok(code)
}

fn id(args: &IdArgs, out: &mut impl Write) -> io::Result<u8> {
    let dir = args.dir.clone().unwrap_or_else(project::specs_dir);
    if !dir.is_dir() {
        eprintln!("Spec directory not found: {}", dir.display());
        return Ok(exit::REFUSED);
    }
    let mut missing = Vec::new();
    for id in &args.ids {
        let hits = spec::find_id(&dir, id)?;
        if hits.is_empty() {
            missing.push(id.as_str());
        }
        for hit in hits {
            writeln!(out, "{}:{}:{}", hit.path.display(), hit.line, hit.text)?;
        }
    }
    if missing.is_empty() {
        return Ok(exit::OK);
    }
    eprintln!(
        "No spec entry found for ID(s): {} (searched {})",
        missing.join(" "),
        dir.display()
    );
    Ok(exit::REFUSED)
}

fn related(args: &RelatedArgs, out: &mut impl Write) -> io::Result<u8> {
    let keywords: Vec<String> = args.keywords.iter().map(|k| k.to_lowercase()).collect();
    let mut found = false;
    for (area, kind, path) in spec::entry_files(&project::specs_dir())? {
        let text = std::fs::read_to_string(&path)?;
        for entry in spec::entries(&text) {
            let low = entry.text.to_lowercase();
            if !keywords.iter().any(|k| low.contains(k.as_str())) {
                continue;
            }
            found = true;
            let heading = entry
                .heading
                .map(|h| format!(" `## {h}`"))
                .unwrap_or_default();
            let text = if entry.text.chars().count() > RELATED_TEXT_MAX {
                let cut: String = entry.text.chars().take(RELATED_TEXT_MAX - 1).collect();
                format!("{cut}…")
            } else {
                entry.text
            };
            writeln!(
                out,
                "{} — {area}/{}{heading} — {text}",
                entry.id,
                kind.file_name()
            )?;
        }
    }
    if found {
        return Ok(exit::OK);
    }
    writeln!(out, "related=none")?;
    Ok(exit::REFUSED)
}

fn git(worktree: &Path, args: &[&str]) -> io::Result<std::process::Output> {
    Command::new("git")
        .arg("-C")
        .arg(worktree)
        .args(args)
        .output()
}

fn defined_in(worktree: &Path, id: &str, tree: &str) -> io::Result<bool> {
    let pattern = format!("^\\*\\*{id}\\*\\*");
    let status = git(
        worktree,
        &["grep", "-q", &pattern, tree, "--", "docs/specs"],
    )?
    .status;
    Ok(status.success())
}

fn collisions(args: &CollisionsArgs, out: &mut impl Write) -> io::Result<u8> {
    let wt = args.worktree.as_path();
    let base = git(wt, &["merge-base", "HEAD", &args.reference])?;
    if !base.status.success() {
        writeln!(out, "ids=error reason=no-merge-base")?;
        return Ok(exit::USAGE);
    }
    let base = String::from_utf8_lossy(&base.stdout).trim().to_owned();
    let diff = git(wt, &["diff", &base, "HEAD", "--", "docs/specs"])?;
    let diff = String::from_utf8_lossy(&diff.stdout);
    let added: BTreeSet<&str> = diff
        .lines()
        .filter_map(|l| l.strip_prefix('+'))
        .filter_map(spec::defined_id)
        .collect();
    let mut hits = Vec::new();
    for id in added {
        if !defined_in(wt, id, &base)? && defined_in(wt, id, &args.reference)? {
            hits.push(id);
        }
    }
    if hits.is_empty() {
        writeln!(out, "ids=clean")?;
        return Ok(exit::OK);
    }
    writeln!(out, "ids=collide {}", hits.join(","))?;
    Ok(exit::REFUSED)
}

fn render(args: &RenderArgs, out: &mut impl Write) -> io::Result<u8> {
    let Some(text) = read(&args.spec_diff) else {
        return Ok(exit::REFUSED);
    };
    let part = match args.part {
        RenderPart::Scenarios => Part::Scenarios,
        RenderPart::Requirements => Part::Requirements,
        RenderPart::Entries => Part::Entries,
        RenderPart::Changes => Part::Changes,
        RenderPart::Deferred => Part::Deferred,
    };
    let issue_ids = if args.issue_ids {
        let run_dir = std::fs::canonicalize(&args.spec_diff)?;
        let slug = run_dir
            .parent()
            .and_then(Path::file_name)
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        Some(match super::card::store(false)? {
            Some(s) => board::reworded_ids(&s.conn, &slug).map_err(super::card::sql)?,
            None => HashSet::new(),
        })
    } else {
        None
    };
    let only = args
        .only
        .as_deref()
        .map(|o| o.split_whitespace().map(str::to_owned).collect::<Vec<_>>())
        .filter(|o| !o.is_empty());
    let opts = spec_diff::Render {
        only,
        first: args.first == 1,
        issue_ids,
    };
    write!(
        out,
        "{}",
        spec_diff::render(&SpecDiff::parse(&text), part, &opts)
    )?;
    Ok(exit::OK)
}
