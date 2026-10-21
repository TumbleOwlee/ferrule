//! The workflow's instructions as topics, printed one at a time and cut to
//! the run they are read for.
//!
//! A topic is markdown. A line `<!-- lane: fast -->` (or `full`) keeps the
//! block below it for that lane only; `<!-- done: <token> -->` marks the
//! block done once the run's parent card carries the token. Below a
//! heading, a tag covers the heading's whole section. `{slug}`, `{stage}`
//! and the other names [`Context::vars`] knows are filled in; unknown ones
//! print as `<slug>`.
//!
//! Defaults ship in the binary. `.ferrule/guide/<topic>.md` overrides one:
//! without headings it replaces the topic, with headings each of its
//! sections replaces the default's section of the same heading, and one
//! the default lacks is appended. An override naming no default topic is a
//! topic of the project's own.

mod render;
mod sections;

use std::path::Path;

use crate::markdown;

use render::tag;
pub use render::{Context, Lane, VARS, render};
pub use sections::{body, headings, merge, select};

macro_rules! topics {
    ($($variant:ident $name:literal,)*) => {
        /// The topics the binary ships.
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum Topic {
            $($variant,)*
        }

        impl Topic {
            pub const ALL: &[Topic] = &[$(Topic::$variant,)*];

            #[must_use]
            pub fn name(self) -> &'static str {
                match self {
                    $(Topic::$variant => $name,)*
                }
            }

            #[must_use]
            pub fn text(self) -> &'static str {
                match self {
                    $(Topic::$variant => include_str!(concat!("../../guide/", $name, ".md")),)*
                }
            }
        }
    };
}

topics! {
    Principles "principles",
    Lanes "lanes",
    Verify "verify",
    Board "board",
    Handoff "handoff",
    Approval "approval",
    Question "question",
    Gate1 "gate1",
    Architecture "architecture",
    Epic "epic",
    Gate2 "gate2",
    Issue "issue",
    Stages "stages",
    StageReview "stage-review",
    Replan "replan",
    Feedback "feedback",
    SpecChanges "spec-changes",
    Split "split",
    Gate3 "gate3",
    Gate4 "gate4",
    Cleanup "cleanup",
    Resume "resume",
    Author "author",
    AuthorLate "author-late",
    Architect "architect",
    ArchitectEpic "architect-epic",
    Planner "planner",
    PlannerReplan "planner-replan",
    Implementer "implementer",
    Reviewer "reviewer",
    ReviewerPlan "reviewer-plan",
    Decisions "decisions",
}

impl Topic {
    #[must_use]
    pub fn parse(name: &str) -> Option<Topic> {
        Topic::ALL.iter().copied().find(|t| t.name() == name)
    }

    /// Read by a spawned agent rather than the orchestrator.
    #[must_use]
    pub fn is_agent(self) -> bool {
        matches!(
            self,
            Topic::Author
                | Topic::AuthorLate
                | Topic::Architect
                | Topic::ArchitectEpic
                | Topic::Planner
                | Topic::PlannerReplan
                | Topic::Implementer
                | Topic::Reviewer
                | Topic::ReviewerPlan
                | Topic::Decisions
        )
    }
}

fn override_path(root: &Path, name: &str) -> std::path::PathBuf {
    root.join(".ferrule")
        .join("guide")
        .join(format!("{name}.md"))
}

/// The topic's text with the project's override applied; None when neither
/// the binary nor the project has it.
#[must_use]
pub fn load(root: &Path, name: &str) -> Option<String> {
    let over = std::fs::read_to_string(override_path(root, name)).ok();
    match (Topic::parse(name), over) {
        (Some(t), Some(o)) => Some(merge(t.text(), &o)),
        (Some(t), None) => Some(t.text().to_owned()),
        (None, Some(o)) => Some(o),
        (None, None) => None,
    }
}

/// The project's own topics: overrides naming no default topic.
fn project_topics(root: &Path) -> Vec<String> {
    let Ok(dir) = std::fs::read_dir(root.join(".ferrule").join("guide")) else {
        return Vec::new();
    };
    let mut names: Vec<String> = dir
        .filter_map(Result::ok)
        .filter_map(|e| {
            e.file_name()
                .to_str()?
                .strip_suffix(".md")
                .map(str::to_owned)
        })
        .collect();
    names.sort();
    names
}

/// A topic's one-line summary: its first line, `# <name> — <summary>`.
fn summary(text: &str) -> &str {
    let first = text.lines().next().unwrap_or("");
    first
        .split_once(" — ")
        .map_or(first.trim_start_matches("# "), |(_, s)| s)
}

/// Every topic, `<name>  <summary>`, under an `Orchestrator:`, `Agents:`
/// and `Project:` heading; overridden ones marked.
#[must_use]
pub fn list(root: &Path) -> Vec<String> {
    let local = project_topics(root);
    let line = |t: &Topic| {
        let text = load(root, t.name()).unwrap_or_default();
        let mark = if local.iter().any(|n| n == t.name()) {
            " (project override)"
        } else {
            ""
        };
        format!("  {:<15} {}{mark}", t.name(), summary(&text))
    };
    let mut out = vec!["Orchestrator:".to_owned()];
    out.extend(Topic::ALL.iter().filter(|t| !t.is_agent()).map(line));
    out.push("Agents:".to_owned());
    out.extend(Topic::ALL.iter().filter(|t| t.is_agent()).map(line));
    let own: Vec<&String> = local.iter().filter(|n| Topic::parse(n).is_none()).collect();
    if !own.is_empty() {
        out.push("Project:".to_owned());
    }
    for name in own {
        let text = load(root, name).unwrap_or_default();
        out.push(format!("  {name:<15} {}", summary(&text)));
    }
    out
}

/// The names `ferrule guide show <name>` mentions in `text`.
fn references(text: &str) -> Vec<&str> {
    text.match_indices("ferrule guide show ")
        .filter_map(|(i, m)| {
            let rest = &text[i + m.len()..];
            let end = rest
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
                .unwrap_or(rest.len());
            let name = &rest[..end];
            (!name.is_empty() && !name.starts_with('-')).then_some(name)
        })
        .collect()
}

/// The `(topic, heading)` pairs of every `ferrule guide show <topic> --section
/// '<heading>'` in `text`; a heading without quotes is one word.
fn section_references(text: &str) -> Vec<(&str, &str)> {
    let mut out = Vec::new();
    for (i, m) in text.match_indices("ferrule guide show ") {
        let rest = &text[i + m.len()..];
        let rest = &rest[..rest.find('`').unwrap_or(rest.len())];
        let name = rest.split_whitespace().next().unwrap_or("");
        let mut args = rest;
        while let Some(at) = args.find("--section ") {
            let value = &args[at + "--section ".len()..];
            let (heading, tail) = match value.strip_prefix('\'') {
                Some(q) => q.split_once('\'').unwrap_or((q, "")),
                None => value.split_once(' ').unwrap_or((value, "")),
            };
            out.push((name, heading));
            args = tail;
        }
    }
    out
}

/// What is wrong with the guide as the project sees it: a topic another
/// names that does not exist, a section reference whose topic lacks the
/// heading, a malformed tag, a project topic without its
/// `# <name> — <summary>` line. Empty when sound.
#[must_use]
pub fn check(root: &Path) -> Vec<String> {
    let local = project_topics(root);
    let mut names: Vec<String> = Topic::ALL.iter().map(|t| t.name().to_owned()).collect();
    names.extend(local.iter().cloned());
    names.sort();
    names.dedup();
    let mut problems = Vec::new();
    for name in &names {
        let text = load(root, name).unwrap_or_default();
        for r in references(&text) {
            if !names.iter().any(|n| n == r) {
                problems.push(format!(
                    "topic={name} names-unknown={r} hint=fix the name or add the topic"
                ));
            }
        }
        for (topic, heading) in section_references(&text) {
            let target = load(root, topic).unwrap_or_default();
            if names.iter().any(|n| n == topic)
                && !headings(&target)
                    .iter()
                    .any(|h| h.eq_ignore_ascii_case(heading))
            {
                problems.push(format!("topic={name} names-unknown={topic}#{heading} hint=add the heading to the topic or fix the reference"));
            }
        }
        for line in markdown::lines(&text) {
            if let Some(Err(bad)) = tag(line) {
                problems.push(format!("topic={name} bad-tag={bad} hint=a tag is <!-- lane: fast|full --> or <!-- done: pr=ready -->"));
            }
        }
        if Topic::parse(name).is_none() && !text.starts_with(&format!("# {name} — ")) {
            problems.push(format!(
                "topic={name} no-summary-line hint=start the file with # {name} — <summary>"
            ));
        }
    }
    problems
}

#[cfg(test)]
mod tests {
    use super::render::Tag;
    use super::sections::sections;
    use super::*;

    const TEXT: &str = "# stages — Implement\n\nBoth lanes.\n\n<!-- lane: fast -->\n## Fast lane\n\nCommit on green.\n\n- one\n- two\n\n<!-- lane: full -->\n## Full lane\n\nStop per rules.\n\n## After\n\n<!-- done: pr=ready -->\nMark it ready for {slug} in {run_dir}, not {owner}.\n";

    fn ctx(lane: Option<Lane>) -> Context {
        Context {
            lane,
            done: vec!["pr=ready".to_owned()],
            vars: vec![("slug", "mrc".to_owned())],
        }
    }

    #[test]
    fn ut_render_cuts_to_the_lane() {
        let fast = render(TEXT, &ctx(Some(Lane::Fast)));
        assert!(
            fast.contains("Commit on green.\n\n- one\n- two\n"),
            "{fast}"
        );
        assert!(
            !fast.contains("Full lane") && !fast.contains("Stop per rules"),
            "{fast}"
        );
        assert!(
            fast.contains("## After"),
            "untagged heading ends the full scope: {fast}"
        );
        assert!(!fast.contains("<!--"), "{fast}");
    }

    #[test]
    fn ut_render_labels_both_lanes_without_one() {
        let both = render(TEXT, &ctx(None));
        assert!(both.contains("## Fast lane (fast lane)\n"), "{both}");
        assert!(both.contains("## Full lane (full lane)\n"), "{both}");
        assert!(both.contains("Stop per rules."), "{both}");
    }

    #[test]
    fn ut_render_marks_done_and_fills_vars() {
        let out = render(TEXT, &ctx(Some(Lane::Full)));
        assert!(
            out.contains("(done)\nMark it ready for mrc in <run_dir>, not {owner}.\n"),
            "{out}"
        );
        let open = render(TEXT, &Context::default());
        assert!(!open.contains("(done)"), "{open}");
    }

    #[test]
    fn ut_render_keeps_fenced_blank_lines_in_one_block() {
        let text = "<!-- lane: fast -->\n```sh\na\n\nb\n```\n\nafter\n";
        let out = render(text, &ctx(Some(Lane::Full)));
        assert_eq!(out, "after\n");
    }

    #[test]
    fn ut_a_tilde_fence_hides_its_headings_and_blank_lines() {
        let text = "<!-- lane: fast -->\n~~~md\n## not a section\n\nb\n~~~\n\nafter\n";
        assert_eq!(render(text, &ctx(Some(Lane::Full))), "after\n");
        let doc = "## A\n\n~~~\n## inside\n~~~\n\n## B\n\nb\n";
        let heads: Vec<String> = sections(doc).into_iter().map(|(h, _, _)| h).collect();
        assert_eq!(heads, ["## A", "## B"]);
    }

    #[test]
    fn ut_has_matches_fields_and_tokens() {
        let c = Context {
            done: vec!["issue=12".to_owned(), "gate2=".to_owned()],
            ..Context::default()
        };
        assert!(c.has("issue") && c.has("issue=12"));
        assert!(!c.has("gate2") && !c.has("pr"));
    }

    #[test]
    fn ut_merge_replaces_and_appends_sections() {
        let default = "# t — x\n\nintro\n\n## A\n\na1\n\n### A.1\n\nsub\n\n## B\n\nb1\n";
        let over = "## A\n\nmine\n\n## C\n\nnew\n";
        assert_eq!(
            merge(default, over),
            "# t — x\n\nintro\n\n## A\n\nmine\n\n## B\n\nb1\n\n## C\n\nnew\n"
        );
        assert_eq!(merge(default, "whole\n"), "whole\n");
    }

    #[test]
    fn ut_references_and_tags() {
        assert_eq!(
            references("run `ferrule guide show gate4 --slug x`, then ferrule guide list"),
            ["gate4"]
        );
        assert_eq!(tag("<!-- lane: fast -->"), Some(Ok(Tag::Lane(Lane::Fast))));
        assert!(matches!(tag("<!-- lane: slow -->"), Some(Err(_))));
        assert_eq!(tag("<!-- note: x -->"), None);
    }

    #[test]
    fn ut_select_body_and_section_references() {
        let text = "# t — x\n\nintro\n\n## A\n\na1\n\n### A.1\n\nsub\n\n## B b\n\nb1\n";
        assert_eq!(
            body(text),
            "intro\n\n## A\n\na1\n\n### A.1\n\nsub\n\n## B b\n\nb1\n"
        );
        assert_eq!(body("## A\n"), "## A\n");
        assert_eq!(
            select(text, &["b B".into(), "a".into()]),
            Ok("## B b\n\nb1\n\n## A\n\na1\n\n### A.1\n\nsub\n\n".into())
        );
        assert_eq!(select(text, &["C".into()]), Err(vec!["C".into()]));
        assert_eq!(
            select(
                "## Fast lane (fast lane, done)\n\nx\n",
                &["fast lane".into()]
            ),
            Ok("## Fast lane (fast lane, done)\n\nx\n".into())
        );
        assert_eq!(headings("## Gate (2)\n"), ["Gate (2)"]);
        assert_eq!(
            section_references(
                "see `ferrule guide show stages --section 'Full lane' --section Output`, `ferrule guide show x`"
            ),
            [("stages", "Full lane"), ("stages", "Output")]
        );
    }

    #[test]
    fn ut_list_groups_agent_topics_last() {
        let dir = tempfile::tempdir().unwrap();
        let list = list(dir.path());
        let agents = list.iter().position(|l| l == "Agents:").unwrap();
        assert_eq!(list[0], "Orchestrator:");
        assert!(
            list[1..agents]
                .iter()
                .any(|l| l.trim_start().starts_with("gate2 "))
        );
        assert!(
            list[agents..]
                .iter()
                .any(|l| l.trim_start().starts_with("implementer "))
        );
        assert!(!list.iter().any(|l| l == "Project:"));
    }

    #[test]
    fn ut_shipped_topics_are_sound() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(check(dir.path()), Vec::<String>::new());
        for t in Topic::ALL {
            assert!(
                t.text().starts_with(&format!("# {} — ", t.name())),
                "{} lacks its summary line",
                t.name()
            );
        }
    }
}
