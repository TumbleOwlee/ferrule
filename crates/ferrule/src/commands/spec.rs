use std::collections::HashSet;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use clap::{Args, Subcommand, ValueEnum};
use ferrule_core::card::board;
use ferrule_core::markdown::{self, Match};
use ferrule_core::spec::diff::{Part, SpecDiff};
use ferrule_core::term::{self, Style};
use ferrule_core::{project, spec};

use super::exit;

#[derive(Debug, Subcommand)]
pub enum SpecCommand {
    /// Print markdown sections by heading.
    ///
    /// A section runs from its heading to the next heading of equal or
    /// shallower depth. Ask for every heading of one file in one call.
    Section(SectionArgs),
    /// List a markdown file's headings.
    Sections(SectionsArgs),
    /// Print spec entries by fixed ID, as `file:line:text`.
    Id(IdArgs),
    /// Print spec entries that mention a keyword.
    Related(RelatedArgs),
    /// List fixed IDs the branch adds that the target ref now has too.
    Collisions(CollisionsArgs),
    /// Render one part of a run's spec-diff.md.
    Render(RenderArgs),
}

#[derive(Debug, Args)]
pub struct SectionArgs {
    /// Match the first heading that starts with each given text.
    #[arg(long)]
    pub prefix: bool,
    /// Heading texts, e.g. `Status`, ignoring case; a leading `## ` also
    /// pins the level.
    #[arg(required = true, num_args = 1..)]
    pub headings: Vec<String>,
    /// The markdown file.
    pub file: PathBuf,
}

#[derive(Debug, Args)]
pub struct SectionsArgs {
    /// Markdown files.
    #[arg(required = true)]
    pub files: Vec<PathBuf>,
}

#[derive(Debug, Args)]
pub struct IdArgs {
    /// Spec directory [default: docs/specs under the project root].
    #[arg(long)]
    pub dir: Option<PathBuf>,
    /// Fixed IDs, requirements and edge cases mixed.
    #[arg(required = true)]
    pub ids: Vec<String>,
}

#[derive(Debug, Args)]
pub struct RelatedArgs {
    /// Keywords, matched as case-insensitive substrings.
    #[arg(required = true)]
    pub keywords: Vec<String>,
}

#[derive(Debug, Args)]
pub struct CollisionsArgs {
    /// The run's worktree, already fetched.
    pub worktree: PathBuf,
    /// The branch the run merges into.
    #[arg(default_value = "origin/main")]
    pub reference: String,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum RenderPart {
    /// `## Scenarios` with Given/When/Then sub-items.
    Scenarios,
    /// `## Requirements` list, local IDs written back.
    Requirements,
    /// `### Requirements` table and `### Other spec changes`.
    Entries,
    /// Dated revision items ending in who decided them.
    Changes,
    /// `### Deferred` list.
    Deferred,
}

#[derive(Debug, Args)]
pub struct RenderArgs {
    /// The run's spec-diff.md.
    pub spec_diff: PathBuf,
    /// The part to render.
    pub part: RenderPart,
    /// Keep only these scenarios and entries, by local or fixed ID,
    /// space-separated.
    #[arg(long, value_name = "IDS")]
    pub only: Option<String>,
    /// With `--only`: also keep "Other spec changes" rows that cite no ID.
    #[arg(long, requires = "only")]
    pub with_unnumbered: bool,
    /// The PR's form of the entries table: an Issue column, and the
    /// requirements reworded after filing.
    ///
    /// Read from the run's cards; the run is the spec diff's directory name.
    #[arg(long)]
    pub issue_ids: bool,
}

/// Longest entry text `spec related` prints before cutting it with `…`.
const RELATED_TEXT_MAX: usize = 100;

pub fn run(command: &SpecCommand, style: Option<Style>, out: &mut impl Write) -> io::Result<u8> {
    match command {
        SpecCommand::Section(args) => section(args, style, out),
        SpecCommand::Sections(args) => sections(args, style, out),
        SpecCommand::Id(args) => id(args, out),
        SpecCommand::Related(args) => related(args, out),
        SpecCommand::Collisions(args) => collisions(args, out),
        SpecCommand::Render(args) => render(args, style, out),
    }
}

fn read(path: &Path) -> Option<String> {
    std::fs::read_to_string(path)
        .inspect_err(|e| eprintln!("cannot read {}: {e}", path.display()))
        .ok()
}

fn section(args: &SectionArgs, style: Option<Style>, out: &mut impl Write) -> io::Result<u8> {
    let Some(text) = read(&args.file) else {
        return Ok(exit::REFUSED);
    };
    let how = if args.prefix {
        Match::Prefix
    } else {
        Match::Exact
    };
    let mut missing = Vec::new();
    let mut printed = false;
    for want in &args.headings {
        let lines = markdown::section_lines(&text, want, how);
        if lines.is_empty() {
            missing.push(want.as_str());
        } else {
            // Trailing blank lines are dropped: a section ends at its last
            // line of content.
            let md = format!("{}\n", lines.join("\n").trim_end_matches('\n'));
            if style.is_some() && printed {
                writeln!(out)?;
            }
            printed = true;
            super::doc(out, style, &md)?;
        }
    }
    if missing.is_empty() {
        return Ok(exit::OK);
    }
    eprintln!("no section matched:\n{}", missing.join("\n"));
    Ok(exit::REFUSED)
}

fn sections(args: &SectionsArgs, style: Option<Style>, out: &mut impl Write) -> io::Result<u8> {
    let mut code = exit::OK;
    for file in &args.files {
        let name = file.display().to_string();
        if args.files.len() > 1 {
            match style {
                Some(s) => writeln!(out, "{}", term::faint(&name, s))?,
                None => writeln!(out, "== {name} ==")?,
            }
        }
        let Some(text) = read(file) else {
            code = exit::REFUSED;
            continue;
        };
        let headings = markdown::headings(&text);
        if let Some(s) = style {
            let tree: Vec<(usize, &str)> = headings
                .iter()
                .map(|h| (h.level, h.line[h.level..].trim()))
                .collect();
            write!(out, "{}", term::outline(&tree, s))?;
            continue;
        }
        for heading in headings {
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

fn collisions(args: &CollisionsArgs, out: &mut impl Write) -> io::Result<u8> {
    match spec::collisions(&args.worktree, &args.reference) {
        None => {
            writeln!(out, "ids=error reason=no-merge-base")?;
            Ok(exit::USAGE)
        }
        Some(hits) if hits.is_empty() => {
            writeln!(out, "ids=clean")?;
            Ok(exit::OK)
        }
        Some(hits) => {
            writeln!(out, "ids=collide {}", hits.join(","))?;
            Ok(exit::REFUSED)
        }
    }
}

fn render(args: &RenderArgs, style: Option<Style>, out: &mut impl Write) -> io::Result<u8> {
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
    let opts = spec::diff::Render {
        only,
        first: args.with_unnumbered,
        issue_ids,
    };
    super::doc(
        out,
        style,
        &spec::diff::render(&SpecDiff::parse(&text), part, &opts),
    )?;
    Ok(exit::OK)
}
