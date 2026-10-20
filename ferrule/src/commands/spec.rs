use std::collections::HashSet;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use clap::{Args, Subcommand, ValueEnum};
use ferrule_core::card::board;
use ferrule_core::config::Config;
use ferrule_core::markdown::{self, Match};
use ferrule_core::spec::diff::{Part, SpecDiff};
use ferrule_core::term::{self, Style};
use ferrule_core::{git, project, spec};

use super::exit;

/// Shown after `ferrule spec id --help`.
const ID_AFTER_LONG_HELP: &str = "\
Output:
  <file>:<line>:<text>           One line per spec line that carries
                                 **<ID>**, files in path order.
Refusals (stderr, exit 1):
  Spec directory not found: <dir>
                                 Pass --dir, or run in a project with
                                 docs/specs.
  No spec entry found for ID(s): <ids> (searched <dir>)
                                 Matched IDs still print. Check the ID's
                                 spelling, e.g. MB-R-012; `ferrule spec
                                 related` finds IDs by keyword.
Exit codes: 0 every ID found, 1 refused, 2 usage.";

/// Shown after `ferrule spec related --help`.
const RELATED_AFTER_LONG_HELP: &str = "\
Output:
  <id> — <area>/<file> `## <heading>` — <text>
                                 One line per requirement or edge case under
                                 docs/specs whose text contains every
                                 keyword (for any of them, run once per
                                 keyword), with the entry's full text; the
                                 heading part is left out when the entry
                                 has none.
No match:
  related=none
                                 No entry matches (stdout, exit 1). Nothing
                                 to cite; try fewer or other nouns, or go on
                                 when none fits.
Too many matches:
  related=too-many count=<n> max=<m> hint=rerun with more or more specific keywords
                                 More than `search_max` entries (in
                                 [thresholds] of ferrule.toml, default 40)
                                 match; none is printed. Every keyword must
                                 match, so add one or make one narrower
                                 (exit 1).
Exit codes: 0 ok, 1 refused, 2 usage.
See: ferrule guide show author";

/// Shown after `ferrule spec collisions --help`.
const COLLISIONS_AFTER_LONG_HELP: &str = "\
Output:
  ids=clean                      The branch adds no fixed ID that REF
                                 defines too.
Refusals:
  ids=collide <id>,… hint=renumber these IDs on the branch
                                 Another run landed these IDs first; resume
                                 the implementer to renumber them on the
                                 branch (spec, test citations, spec-delta
                                 log), then check again.
  ids=error reason=no-merge-base hint=fetch the REF, or pass a REF the branch
                                 descends from
                                 The branch and REF share no history (exit
                                 2). Fetch REF, or name the right one.
Exit codes: 0 clean, 1 collide, 2 error or usage.
See: ferrule guide show gate4";

/// Shown after `ferrule spec next --help`.
const NEXT_AFTER_LONG_HELP: &str = "\
Output, one line:
  next=<ID>[,<ID>…] last=<ID|->  The next free fixed ID(s) of the prefix and
                                 series, one above the highest found; last
                                 is that highest, `-` when there is none.
Where it looks: every `<PREFIX>-R-<n>` (or `-E-` with --edge) mentioned under
docs/specs on REF (default: main_branch of ferrule.toml), never the working
tree. With --run <slug>, also the run's worktree docs/specs, its
spec-diff.md and every `spec-delta <ID> added` on its cards, so a number
dropped earlier in the run is not reused.
Refusals (exit 1):
  next=refused reason=prefix hint=a prefix is two capital letters, e.g. MB
  next=refused reason=ref hint=fetch REF or pass --ref
  next=refused reason=card hint=check the slug with ferrule board list
Exit codes: 0 ok, 1 refused, 2 usage.
Examples:
  ferrule spec next MB --count 3
  ferrule spec next MB --edge --run <slug>
See: ferrule guide show planner";

/// Shown after `ferrule spec refs --help`.
const REFS_AFTER_LONG_HELP: &str = "\
Output:
  <path>:<line>:<text>           Every tracked line citing one of the IDs as a
                                 whole word, in git grep order; whole lines.
Where it looks: the current checkout's tracked files (the worktree when run
inside it), or REF with --ref; --path narrows to pathspecs (repeat it), e.g.
`--path docs/specs --path tests`.
No match:
  refs=none                      Nothing cites any of the IDs (exit 1). Before
                                 a new ID lands, that is the wanted answer.
Refusals:
  refs=too-many count=<n> max=<m> hint=pass fewer IDs or narrow with --path
                                 More than `search_max` lines ([thresholds]
                                 of ferrule.toml, default 40); none printed.
  refs=refused reason=id id=<word> hint=a fixed ID looks like MB-R-001
                                 A word is not a fixed ID (exit 2).
  refs=refused reason=ref hint=fetch REF or pass another --ref
Exit codes: 0 ok, 1 none or refused, 2 usage.
Examples:
  ferrule spec refs MB-R-288 --path src --path tests
  ferrule spec refs MB-R-301 --ref main --path docs/specs
See: ferrule guide show reviewer";

/// Shown after `ferrule spec render --help`.
const RENDER_AFTER_LONG_HELP: &str = "\
Output:
  The part as markdown, filtered by --only. Prints nothing for a part the
  spec-diff does not have. No status line.
Refusals (stderr, exit 1):
  cannot read <file>: <error>    The spec-diff.md is missing or unreadable.
Exit codes: 0 ok, 1 refused, 2 usage.
Example:
  ferrule spec render .ferrule/runs/<slug>/spec-diff.md entries --only 'R1 R3'";

#[derive(Debug, Subcommand)]
pub enum SpecCommand {
    /// Print spec entries by fixed ID, as `file:line:text`.
    #[command(after_long_help = ID_AFTER_LONG_HELP)]
    Id(IdArgs),
    /// Print spec entries that mention every keyword.
    #[command(after_long_help = RELATED_AFTER_LONG_HELP)]
    Related(RelatedArgs),
    /// List fixed IDs the branch adds that the target ref now has too.
    ///
    /// Prints `ids=clean` (exit 0), `ids=collide <id>,…` (exit 1), or
    /// `ids=error` when the two share no merge base (exit 2).
    #[command(after_long_help = COLLISIONS_AFTER_LONG_HELP)]
    Collisions(CollisionsArgs),
    /// Render one part of a run's spec-diff.md.
    #[command(after_long_help = RENDER_AFTER_LONG_HELP)]
    Render(RenderArgs),
    /// Print the next free fixed ID of a prefix.
    #[command(after_long_help = NEXT_AFTER_LONG_HELP)]
    Next(NextArgs),
    /// Print every line that cites the IDs, in code, tests and specs.
    #[command(after_long_help = REFS_AFTER_LONG_HELP)]
    Refs(RefsArgs),
}

#[derive(Debug, Args)]
pub struct NextArgs {
    /// The area's prefix, two capital letters: `MB` of `MB-R-001`.
    pub prefix: String,
    /// The edge-case series, `-E-`, instead of requirements, `-R-`.
    #[arg(long)]
    pub edge: bool,
    /// How many IDs to hand out, counting up.
    #[arg(long, value_name = "N", default_value_t = 1, value_parser = clap::value_parser!(u32).range(1..))]
    pub count: u32,
    /// Also count the run's worktree, spec-diff.md and `spec-delta` lines.
    #[arg(long, value_name = "SLUG")]
    pub run: Option<String>,
    /// The ref whose docs/specs counts [default: `main_branch` of ferrule.toml].
    #[arg(long, value_name = "REF")]
    pub r#ref: Option<String>,
}

#[derive(Debug, Args)]
pub struct RefsArgs {
    /// Fixed IDs, e.g. `MB-R-001`.
    #[arg(required = true, value_name = "ID")]
    pub ids: Vec<String>,
    /// Only files under this pathspec; repeat for several.
    #[arg(long, value_name = "PATH")]
    pub path: Vec<String>,
    /// Search this ref instead of the checkout, e.g. `main`.
    #[arg(long, value_name = "REF")]
    pub r#ref: Option<String>,
}

#[derive(Debug, Args)]
pub struct SectionArgs {
    /// Match the first heading that starts with each given text.
    #[arg(long)]
    pub prefix: bool,
    /// Heading texts, e.g. `Status`, ignoring case; a leading `## ` also
    /// pins the level.
    #[arg(required = true, num_args = 1.., value_name = "HEADING")]
    pub headings: Vec<String>,
    /// The markdown file, after the headings.
    pub file: PathBuf,
}

#[derive(Debug, Args)]
pub struct SectionsArgs {
    /// Markdown files.
    #[arg(required = true, value_name = "FILE")]
    pub files: Vec<PathBuf>,
    /// Only headings of this level or above: 2 keeps `#` and `##`.
    #[arg(long, value_name = "N", value_parser = clap::value_parser!(u8).range(1..=6))]
    pub depth: Option<u8>,
}

#[derive(Debug, Args)]
pub struct IdArgs {
    /// Spec directory [default: docs/specs under the project root].
    #[arg(long)]
    pub dir: Option<PathBuf>,
    /// Fixed IDs, requirements and edge cases mixed.
    #[arg(required = true, value_name = "ID")]
    pub ids: Vec<String>,
}

#[derive(Debug, Args)]
pub struct RelatedArgs {
    /// Keywords, matched as case-insensitive substrings; an entry is
    /// listed when it contains every one of them.
    #[arg(required = true, value_name = "KEYWORD")]
    pub keywords: Vec<String>,
}

#[derive(Debug, Args)]
pub struct CollisionsArgs {
    /// The run's worktree, already fetched.
    pub worktree: PathBuf,
    /// The branch the run merges into [default: `origin/<main_branch>` of
    /// `ferrule.toml`].
    #[arg(value_name = "REF")]
    pub reference: Option<String>,
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
    #[arg(value_name = "SPEC_DIFF_MD")]
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

pub fn run(command: &SpecCommand, style: Option<Style>, out: &mut impl Write) -> io::Result<u8> {
    match command {
        SpecCommand::Id(args) => id(args, out),
        SpecCommand::Related(args) => related(args, out),
        SpecCommand::Collisions(args) => collisions(args, out),
        SpecCommand::Render(args) => render(args, style, out),
        SpecCommand::Next(args) => next(args, out),
        SpecCommand::Refs(args) => refs(args, out),
    }
}

fn next(args: &NextArgs, out: &mut impl Write) -> io::Result<u8> {
    let prefix = &args.prefix;
    if prefix.len() != 2 || !prefix.bytes().all(|b| b.is_ascii_uppercase()) {
        writeln!(
            out,
            "next=refused reason=prefix hint=a prefix is two capital letters, e.g. MB"
        )?;
        return Ok(exit::REFUSED);
    }
    let series = if args.edge { 'E' } else { 'R' };
    let root = project::main_checkout();
    let config = Config::load(&root).map_err(super::board::sql)?;
    let reference = args.r#ref.clone().unwrap_or(config.main_branch);
    if git::commit(&root, &reference).is_none() {
        writeln!(out, "next=refused reason=ref hint=fetch REF or pass --ref")?;
        return Ok(exit::REFUSED);
    }
    // the leading class keeps `XMB-R-1` out, which `-o` would cut to `MB-R-1`
    let pattern = format!("(^|[^A-Za-z0-9]){prefix}-{series}-[0-9]+");
    let mut texts = vec![
        git::out(
            &root,
            &["grep", "-hoE", &pattern, &reference, "--", "docs/specs"],
        )
        .unwrap_or_default(),
    ];
    if let Some(slug) = &args.run {
        let Some(found) = run_texts(slug)? else {
            writeln!(
                out,
                "next=refused reason=card hint=check the slug with ferrule board list"
            )?;
            return Ok(exit::REFUSED);
        };
        texts.extend(found);
    }
    let best = texts
        .iter()
        .filter_map(|t| spec::highest(prefix, series, t))
        .max_by_key(|(n, _)| *n);
    let width = best.map_or(3, |(_, w)| w.max(3));
    let id = |n: u32| format!("{prefix}-{series}-{n:0width$}");
    let first = best.map_or(1, |(n, _)| n + 1);
    let ids: Vec<String> = (first..first + args.count).map(id).collect();
    let last = best.map_or_else(|| "-".to_owned(), |(n, _)| id(n));
    writeln!(out, "next={} last={last}", ids.join(","))?;
    Ok(exit::OK)
}

/// The run's worktree specs, spec-diff.md and `spec-delta … added` lines;
/// `None` when the run has no card.
fn run_texts(slug: &str) -> io::Result<Option<Vec<String>>> {
    let layout = project::Layout::new(project::main_checkout());
    let Some(s) = super::board::store(false)? else {
        return Ok(None);
    };
    if board::state(&s.conn, slug)
        .map_err(super::board::sql)?
        .is_none()
    {
        return Ok(None);
    }
    let mut texts = Vec::new();
    let wt = board::field(&s.conn, slug, "worktree")
        .map_err(super::board::sql)?
        .map_or_else(|| layout.worktree(slug), |w| layout.resolve(&w));
    let specs = wt.join("docs/specs");
    if specs.is_dir() {
        for path in spec::markdown_files(&specs)? {
            texts.push(std::fs::read_to_string(path)?);
        }
    }
    if let Ok(diff) = std::fs::read_to_string(layout.run_dir(slug).join("spec-diff.md")) {
        texts.push(diff);
    }
    for (_, _, body) in board::run_events(&s.conn, slug).map_err(super::board::sql)? {
        let mut words = body.split_whitespace();
        if words.next() == Some("spec-delta") && words.nth(1) == Some("added") {
            texts.push(body);
        }
    }
    Ok(Some(texts))
}

fn refs(args: &RefsArgs, out: &mut impl Write) -> io::Result<u8> {
    if let Some(bad) = args.ids.iter().find(|id| !spec::is_fixed_id(id)) {
        writeln!(
            out,
            "refs=refused reason=id id={bad} hint=a fixed ID looks like MB-R-001"
        )?;
        return Ok(exit::USAGE);
    }
    let dir = if args.r#ref.is_some() {
        project::main_checkout()
    } else {
        project::root()
    };
    if let Some(r) = &args.r#ref
        && git::commit(&dir, r).is_none()
    {
        writeln!(
            out,
            "refs=refused reason=ref hint=fetch REF or pass another --ref"
        )?;
        return Ok(exit::REFUSED);
    }
    let mut cmd: Vec<&str> = vec!["grep", "-n", "-w", "-F", "--no-color"];
    for id in &args.ids {
        cmd.extend(["-e", id]);
    }
    cmd.extend(args.r#ref.as_deref());
    cmd.push("--");
    cmd.extend(args.path.iter().map(String::as_str));
    let text = git::out(&dir, &cmd).unwrap_or_default();
    let strip = args.r#ref.as_ref().map(|r| format!("{r}:"));
    let lines: Vec<&str> = text
        .lines()
        .map(|l| {
            strip
                .as_deref()
                .and_then(|p| l.strip_prefix(p))
                .unwrap_or(l)
        })
        .collect();
    let max = Config::load(&project::main_checkout())
        .map_err(super::board::sql)?
        .thresholds
        .search_max;
    if lines.len() > max {
        writeln!(
            out,
            "refs=too-many count={} max={max} hint=pass fewer IDs or narrow with --path",
            lines.len()
        )?;
        return Ok(exit::REFUSED);
    }
    if lines.is_empty() {
        writeln!(out, "refs=none")?;
        return Ok(exit::REFUSED);
    }
    for line in lines {
        writeln!(out, "{line}")?;
    }
    Ok(exit::OK)
}

fn read(path: &Path) -> Option<String> {
    std::fs::read_to_string(path)
        .inspect_err(|e| eprintln!("cannot read {}: {e}", path.display()))
        .ok()
}

pub(super) fn section(
    args: &SectionArgs,
    style: Option<Style>,
    out: &mut impl Write,
) -> io::Result<u8> {
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

pub(super) fn sections(
    args: &SectionsArgs,
    style: Option<Style>,
    out: &mut impl Write,
) -> io::Result<u8> {
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
        let mut headings = markdown::headings(&text);
        if let Some(d) = args.depth {
            headings.retain(|h| h.level <= usize::from(d));
        }
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
    let mut hits = Vec::new();
    for (area, kind, path) in spec::entry_files(&project::specs_dir())? {
        let text = std::fs::read_to_string(&path)?;
        for entry in spec::entries(&text) {
            let low = entry.text.to_lowercase();
            if !keywords.iter().all(|k| low.contains(k.as_str())) {
                continue;
            }
            let heading = entry
                .heading
                .map(|h| format!(" `## {h}`"))
                .unwrap_or_default();
            hits.push(format!(
                "{} — {area}/{}{heading} — {}",
                entry.id,
                kind.file_name(),
                entry.text
            ));
        }
    }
    let max = Config::load(&project::main_checkout())
        .map_err(super::board::sql)?
        .thresholds
        .search_max;
    if hits.len() > max {
        writeln!(
            out,
            "related=too-many count={} max={max} hint=rerun with more or more specific keywords",
            hits.len()
        )?;
        return Ok(exit::REFUSED);
    }
    for hit in &hits {
        writeln!(out, "{hit}")?;
    }
    if !hits.is_empty() {
        return Ok(exit::OK);
    }
    writeln!(out, "related=none")?;
    Ok(exit::REFUSED)
}

fn collisions(args: &CollisionsArgs, out: &mut impl Write) -> io::Result<u8> {
    let reference = if let Some(r) = &args.reference {
        r.clone()
    } else {
        let config = Config::load(&project::main_checkout()).map_err(super::board::sql)?;
        format!("origin/{}", config.main_branch)
    };
    match spec::collisions(&args.worktree, &reference) {
        None => {
            writeln!(
                out,
                "ids=error reason=no-merge-base hint=fetch the REF, or pass a REF the branch descends from"
            )?;
            Ok(exit::USAGE)
        }
        Some(hits) if hits.is_empty() => {
            writeln!(out, "ids=clean")?;
            Ok(exit::OK)
        }
        Some(hits) => {
            writeln!(
                out,
                "ids=collide {} hint=renumber these IDs on the branch",
                hits.join(",")
            )?;
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
        Some(match super::board::store(false)? {
            Some(s) => board::reworded_ids(&s.conn, &slug).map_err(super::board::sql)?,
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
