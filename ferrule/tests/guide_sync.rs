//! The guides, the installed agent files and the guard's replacement hints
//! name `ferrule` commands and shell commands; each must exist in the CLI
//! and pass the shell guard, so text cannot outlive what it points at.

mod golden;

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use ferrule_core::hook::guard::{self, Policy};
use golden::Run;

/// Programs whose backticked uses in the guides must pass the shell guard.
const PROGRAMS: &[&str] = &[
    "ferrule", "git", "gh", "cp", "mv", "rm", "cat", "grep", "rg", "sed", "ls", "cargo", "npm",
    "sh", "bash", "eval", "cd", "find", "xargs", "awk",
];

/// Counter-examples: commands the text tells an agent never to run.
const REFUSED: &[&str] = &["git add -A", "git commit", "git checkout"];

const AGENTS: &[&[&str]] = &[
    &["author", "author-late"],
    &["architect", "architect-epic"],
    &["planner", "planner-replan"],
    &["implementer"],
    &["reviewer", "reviewer-plan"],
];

/// Every guide an agent reads, flattened.
fn agent_guides() -> Vec<&'static str> {
    AGENTS
        .iter()
        .flat_map(|family| family.iter().copied())
        .collect()
}

/// The topic of each `ferrule guide show <topic>` in `text`.
fn citations(text: &str) -> Vec<String> {
    let pattern = "ferrule guide show ";
    text.match_indices(pattern)
        .map(|(at, _)| {
            text[at + pattern.len()..]
                .chars()
                .take_while(|c| c.is_ascii_lowercase() || *c == '-')
                .collect()
        })
        .collect()
}

fn cites_agent_guide(text: &str) -> bool {
    citations(text)
        .iter()
        .any(|c| agent_guides().contains(&c.as_str()))
}

/// Every command path under `path`, `path` included.
fn command_paths(help: &mut Help, path: &[&str], out: &mut Vec<Vec<String>>) {
    out.push(path.iter().map(|s| (*s).to_owned()).collect());
    for sub in help.of(path).1.clone() {
        let mut next = path.to_vec();
        next.push(&sub);
        command_paths(help, &next, out);
    }
}

fn core() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../ferrule-core")
}

fn markdown(dir: &Path, out: &mut Vec<(String, String)>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            markdown(&path, out);
        } else if path.extension().is_some_and(|e| e == "md") {
            let name = path.strip_prefix(core()).unwrap().display().to_string();
            out.push((name, fs::read_to_string(&path).unwrap()));
        }
    }
}

/// Every guide topic and installed file, raw: lane and done tags hide no
/// text from a scan of the source.
fn sources() -> Vec<(String, String)> {
    let mut out = Vec::new();
    markdown(&core().join("guide"), &mut out);
    markdown(&core().join("install"), &mut out);
    assert!(out.len() > 30, "{} files", out.len());
    out
}

/// The backticked spans of `text`, and the lines inside fenced blocks.
fn spans(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut fence = false;
    for line in text.lines() {
        if line.starts_with("```") {
            fence = !fence;
        } else if fence {
            out.push(line.trim().to_owned());
        } else if !line.contains("``") {
            out.extend(
                line.split('`')
                    .skip(1)
                    .step_by(2)
                    .map(|s| s.trim().to_owned()),
            );
        }
    }
    out
}

/// A command path's `--help`, run once.
struct Help(HashMap<String, (String, Vec<String>)>);

impl Help {
    /// The help text and the subcommands of `path`.
    fn of(&mut self, path: &[&str]) -> &(String, Vec<String>) {
        self.0.entry(path.join(" ")).or_insert_with(|| {
            let out = Run::new(path.iter().copied().chain(["--help"])).output();
            let text = String::from_utf8_lossy(&out.stdout).into_owned();
            let subs = text
                .lines()
                .skip_while(|l| *l != "Commands:")
                .skip(1)
                .take_while(|l| !l.is_empty())
                .filter_map(|l| l.split_whitespace().next())
                .filter(|name| *name != "help")
                .map(str::to_owned)
                .collect();
            (text, subs)
        })
    }
}

fn is_word(t: &str) -> bool {
    t.chars().next().is_some_and(|c| c.is_ascii_lowercase())
        && t.chars().all(|c| c.is_ascii_lowercase() || c == '-')
}

/// The `--flag` a token names, if it does.
fn flag(token: &str) -> Option<&str> {
    let rest = token.trim_start_matches(['[', '(']).strip_prefix("--")?;
    let end = rest
        .find(|c: char| !(c.is_ascii_lowercase() || c == '-'))
        .unwrap_or(rest.len());
    (end > 0).then(|| &rest[..end])
}

/// Whether `help` lists `--name` as an option.
fn lists(help: &str, name: &str) -> bool {
    let flag = format!("--{name}");
    help.match_indices(&flag).any(|(i, _)| {
        !help[i + flag.len()..].starts_with(|c: char| c.is_ascii_lowercase() || c == '-')
    })
}

/// What is wrong with the `ferrule …` mentions in `text`.
fn check_mentions(help: &mut Help, text: &str, problems: &mut Vec<String>) {
    let mut rest = text;
    while let Some(i) = rest.find("ferrule ") {
        let before = rest[..i].chars().last();
        let after = &rest[i + "ferrule ".len()..];
        rest = after;
        if before.is_some_and(|c| c.is_alphanumeric() || "-_./".contains(c)) {
            continue;
        }
        let end = after.find("ferrule ").unwrap_or(after.len());
        let tokens: Vec<&str> = after[..end]
            .split_whitespace()
            .map(|t| t.trim_end_matches([',', ';', ')', '.']))
            .collect();
        let mut path: Vec<&str> = Vec::new();
        let mut n = 0;
        while let Some(t) = tokens.get(n) {
            if !help.of(&path).1.iter().any(|s| s == t) {
                break;
            }
            path.push(t);
            n += 1;
        }
        let at = path.join(" ");
        if let Some(t) = tokens.get(n)
            && is_word(t)
            && !help.of(&path).1.is_empty()
        {
            problems.push(format!("no subcommand `{t}` after `ferrule {at}`: {text}"));
            continue;
        }
        let text_help = help.of(&path).0.clone();
        for name in tokens[n..].iter().filter_map(|t| flag(t)) {
            if name != "help" && !lists(&text_help, name) {
                problems.push(format!("`ferrule {at}` has no --{name}: {text}"));
            }
        }
    }
}

#[test]
fn it_ferrule_commands_in_the_text_exist() {
    let mut help = Help(HashMap::new());
    let mut problems = Vec::new();
    for (name, text) in sources() {
        for span in spans(&text) {
            let mut found = Vec::new();
            check_mentions(&mut help, &span, &mut found);
            problems.extend(found.into_iter().map(|p| format!("{name}: {p}")));
        }
    }
    for r in guard::REPLACED {
        let mut found = Vec::new();
        check_mentions(&mut help, r.instead, &mut found);
        problems.extend(
            found
                .into_iter()
                .map(|p| format!("REPLACED {}: {p}", r.raw)),
        );
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn it_shell_commands_in_the_guides_pass_the_guard() {
    let dir = tempfile::tempdir().unwrap();
    let policy = Policy {
        allow: &[],
        rebasing: &[],
    };
    // The guides tell an agent to run these only inside a stopped rebase.
    let stopped = [dir.path().to_path_buf()];
    let in_rebase = Policy {
        allow: &[],
        rebasing: &stopped,
    };
    let mut problems = Vec::new();
    let mut checked = 0;
    for (name, text) in sources() {
        for span in spans(&text) {
            let policy = if [
                "GIT_EDITOR=true git rebase --continue",
                "git rebase --continue",
                "git rebase --abort",
            ]
            .iter()
            .any(|p| span.starts_with(p))
            {
                in_rebase
            } else {
                policy
            };
            let first = span.split_whitespace().next().unwrap_or("");
            let command = span.replace(['<', '>'], "");
            if !PROGRAMS.contains(&first) || command.contains('…') {
                continue;
            }
            checked += 1;
            if REFUSED.contains(&span.as_str()) {
                continue;
            }
            if let Some(reason) = guard::shell(&command, dir.path(), policy) {
                problems.push(format!("{name}: `{span}` is refused: {reason}"));
            }
        }
    }
    assert!(checked > 100, "{checked} commands checked");
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn it_slash_commands_and_scripts_in_the_text_exist() {
    let skills: Vec<String> = fs::read_dir(core().join("install/claude/skills"))
        .unwrap()
        .map(|e| e.unwrap().path().file_stem().unwrap().display().to_string())
        .collect();
    assert!(!skills.is_empty());
    let mut problems = Vec::new();
    for (name, text) in sources() {
        for span in spans(&text) {
            let first = span.split_whitespace().next().unwrap_or("");
            if Path::new(first).extension().is_some_and(|e| e == "sh") {
                problems.push(format!(
                    "{name}: `{span}` runs a script ferrule does not ship"
                ));
            } else if let Some(s) = first.strip_prefix('/').filter(|s| is_word(s)) {
                // `/workflow` is another tool's skill, named to say it is replaced.
                if s != "workflow" && !skills.iter().any(|k| k == s) {
                    problems.push(format!("{name}: `{span}` is not an installed skill"));
                }
            }
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// GA-R-021, GA-R-022 — a guide states a rule rather than citing a heading or a rule number for it.
#[test]
fn it_guides_cite_no_agents_heading_or_readme_rule() {
    const CITATIONS: &[&str] = &[
        "`AGENTS.md` `## ",
        "`## Spec-driven`",
        "`## Conventions — GitHub`",
        "`AGENTS.md` routing table",
        "README.md` rule ",
        "per docs/specs/README.md",
        "`## Workflow`",
        "README rule ",
    ];
    let mut problems = Vec::new();
    for (name, text) in sources() {
        for citation in CITATIONS {
            if text.contains(citation) {
                problems.push(format!("{name}: {citation}"));
            }
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// GA-R-029, GA-R-030 — a guide states what its own audience needs rather than citing another audience's guide.
#[test]
fn it_guides_cite_no_other_audiences_guide() {
    let mut guides = Vec::new();
    markdown(&core().join("guide"), &mut guides);
    let mut problems = Vec::new();
    for (name, text) in guides {
        let topic = name.trim_start_matches("guide/").trim_end_matches(".md");
        let family = AGENTS.iter().find(|family| family.contains(&topic));
        for cited in citations(&text) {
            let refused = match family {
                Some(family) => !family.contains(&cited.as_str()),
                None => AGENTS.iter().any(|family| family.contains(&cited.as_str())),
            };
            if refused {
                problems.push(format!("{name}: {cited}"));
            }
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// One field of two different files' shapes, each inside a fence an include cannot reach.
const SHARED_SHAPE_FIELD: &str = "Open: <decision the user still owes, or none>";

/// GA-R-050, GA-R-055, GA-R-056, GA-R-057 — a shared text lives in one file; no other guide source repeats its lines.
#[test]
fn it_shared_text_has_one_source() {
    let mut guides = Vec::new();
    markdown(&core().join("guide"), &mut guides);
    let mut problems = Vec::new();
    for (name, text) in guides
        .iter()
        .filter(|(n, _)| n.starts_with("guide/shared/"))
    {
        for line in text
            .lines()
            .filter(|l| l.chars().count() >= 40 && *l != SHARED_SHAPE_FIELD)
        {
            for (other, body) in guides.iter().filter(|(n, _)| n != name) {
                if body.lines().any(|l| l == line) {
                    problems.push(format!("{other} repeats a line of {name}: {line}"));
                }
            }
        }
    }
    for (needle, home) in [
        ("- <choice> (", "guide/shared/decisions.md"),
        (
            "**Concise, compact, facts only.**",
            "guide/shared/concise.md",
        ),
        (
            "its assertions directly verify",
            "guide/shared/id-citation.md",
        ),
    ] {
        for (name, text) in &guides {
            if name != home && text.contains(needle) {
                problems.push(format!("{name} holds {needle}"));
            }
        }
        assert!(
            guides.iter().any(|(n, t)| n == home && t.contains(needle)),
            "{home} lacks {needle}"
        );
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// GA-R-047 — an agent file names one guide for each of its jobs.
#[test]
fn it_agent_files_name_one_guide_per_job() {
    let mut files = Vec::new();
    markdown(&core().join("install/claude/agents"), &mut files);
    for (name, text) in &files {
        assert!(!text.contains("also `ferrule guide show"), "{name}");
    }
    for agent in ["fe-author", "fe-architect", "fe-planner", "fe-reviewer"] {
        let (_, text) = files
            .iter()
            .find(|(n, _)| n.ends_with(&format!("{agent}.md")))
            .unwrap();
        assert!(text.contains("` instead)"), "{agent}");
    }
}

/// GA-R-051 — a job guide shares no raw line with its agent's main guide.
#[test]
fn it_job_guides_share_no_text_with_their_main_guide() {
    let read = |n: &str| fs::read_to_string(core().join(format!("guide/{n}.md"))).unwrap();
    let mut problems = Vec::new();
    for family in AGENTS {
        let [main, job] = family else { continue };
        let main_text = read(main);
        for line in read(job).lines() {
            if line.chars().count() >= 40
                && !line.starts_with("<!-- include:")
                && main_text.lines().any(|l| l == line)
            {
                problems.push(format!("{job} repeats a line of {main}: {line}"));
            }
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// GA-R-039 — no command's help cites an agent's guide.
#[test]
fn it_command_help_cites_no_agents_guide() {
    let mut help = Help(HashMap::new());
    let mut paths = Vec::new();
    command_paths(&mut help, &[], &mut paths);
    let mut seen = 0;
    let mut problems = Vec::new();
    for path in &paths {
        let args: Vec<&str> = path.iter().map(String::as_str).collect();
        let text = help.of(&args).0.clone();
        if text.contains("See: ferrule guide show") {
            seen += 1;
        }
        if cites_agent_guide(&text) {
            problems.push(path.join(" "));
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
    assert!(seen >= 20, "{seen} helps hold a See line");
}

/// GA-R-040 — `stage commit` cites the `stages` topic.
#[test]
fn it_stage_commit_help_cites_the_stages_topic() {
    let mut help = Help(HashMap::new());
    let text = &help.of(&["stage", "commit"]).0;
    assert!(text.contains("See: ferrule guide show stages"), "{text}");
}

/// GA-R-041 — six commands carry no `See:` line and keep their own facts.
#[test]
fn it_six_commands_have_no_see_line() {
    let mut help = Help(HashMap::new());
    for (path, refusal) in [
        (["run", "diff"], "diff=refused"),
        (["run", "log"], "log=refused"),
        (["run", "plan-diff"], "plan-diff=refused"),
        (["spec", "related"], "related=too-many"),
        (["spec", "next"], "next=refused"),
        (["spec", "refs"], "refs=refused"),
    ] {
        let text = &help.of(&path).0;
        let name = path.join(" ");
        assert!(!text.contains("See:"), "{name}: {text}");
        let output = if name == "spec next" {
            "Output, one line:"
        } else {
            "Output"
        };
        for needle in ["Exit codes:", output, refusal] {
            assert!(text.contains(needle), "{name} lacks {needle}: {text}");
        }
    }
}

/// GA-R-042, GA-E-001 — only `fe-review` cites an agent guide, the reviewer's.
#[test]
fn it_skills_cite_no_agents_guide() {
    let mut skills = Vec::new();
    markdown(&core().join("install/claude/skills"), &mut skills);
    assert!(skills.len() >= 4, "{} skills", skills.len());
    for (name, text) in &skills {
        let cited: Vec<String> = citations(text)
            .into_iter()
            .filter(|c| agent_guides().contains(&c.as_str()))
            .collect();
        if name.ends_with("fe-review.md") {
            assert!(
                !cited.is_empty() && cited.iter().all(|c| c == "reviewer"),
                "{name}: {cited:?}"
            );
        } else {
            assert!(cited.is_empty(), "{name}: {cited:?}");
        }
    }
}

/// GA-R-043 — the README cites no agent's guide and keeps the stage table.
#[test]
fn it_readme_cites_no_agents_guide() {
    let readme =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../README.md")).unwrap();
    assert!(!cites_agent_guide(&readme));
    assert!(readme.contains("| Stage | Title"));
}

/// GA-R-058 — no installed-file paragraph of two sentences or more is also
/// a paragraph of a guide source. A one-sentence paragraph may stay copied.
#[test]
fn it_installed_files_share_no_paragraph_with_guides() {
    let paragraphs = |dir: &str| {
        let mut files = Vec::new();
        markdown(&core().join(dir), &mut files);
        files
            .into_iter()
            .flat_map(|(name, text)| {
                text.split("\n\n")
                    .map(|p| (name.clone(), p.trim().to_owned()))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>()
    };
    let guides = paragraphs("guide");
    for (name, p) in paragraphs("install/claude") {
        if !p.contains(". ") {
            continue;
        }
        let held: Vec<&String> = guides
            .iter()
            .filter(|(_, g)| *g == p)
            .map(|(n, _)| n)
            .collect();
        assert!(held.is_empty(), "{name} copies a paragraph of {held:?}");
    }
}

/// Every shared text is held by at least two sources.
#[test]
fn it_every_fragment_has_two_holders() {
    let all = sources();
    let mut seen = 0;
    for entry in fs::read_dir(core().join("guide/shared")).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_stem().unwrap().to_str().unwrap().to_owned();
        let line = format!("<!-- include: {name} -->");
        let holders = all
            .iter()
            .filter(|(_, t)| t.lines().any(|l| l.trim() == line))
            .count();
        assert!(holders >= 2, "{name} is held by {holders}");
        seen += 1;
    }
    assert!(seen >= 17, "{seen} fragments");
}

/// GA-R-059 — no heading section of a guide or installed-file source repeats
/// another's body, or holds another's multi-paragraph body. A repeated
/// sentence may stay copied.
#[test]
fn it_heading_sections_have_one_source() {
    let mut sections: Vec<(String, String)> = Vec::new();
    for (name, text) in sources() {
        let mut current: Option<(String, Vec<&str>)> = None;
        let mut done = Vec::new();
        for line in ferrule_core::markdown::classified(&text) {
            let hashes = line.text.chars().take_while(|c| *c == '#').count();
            let heading =
                !line.fenced && (1..=6).contains(&hashes) && line.text[hashes..].starts_with(' ');
            if heading {
                done.extend(current.take());
                current = Some((format!("{name}: {}", line.text), Vec::new()));
            } else if let Some((_, body)) = current.as_mut()
                && !line.text.starts_with("<!-- include:")
            {
                body.push(line.text);
            }
        }
        done.extend(current);
        for (id, body) in done {
            let body = body.join("\n").trim().to_owned();
            if !body.is_empty() {
                sections.push((id, body));
            }
        }
    }
    assert!(sections.len() > 100, "{} sections", sections.len());
    let mut problems = Vec::new();
    for (i, (a_id, a)) in sections.iter().enumerate() {
        for (b_id, b) in &sections[i + 1..] {
            let nested = (a.contains("\n\n") && b.contains(a.as_str()))
                || (b.contains("\n\n") && a.contains(b.as_str()));
            if a == b || nested {
                problems.push(format!("{a_id} and {b_id} repeat text"));
            }
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}
