//! Texts shared by several topics, kept once under `guide/shared/` and
//! spliced into a topic wherever it holds an include line.

use crate::markdown;

macro_rules! fragments {
    ($($variant:ident $name:literal,)*) => {
        /// The shared texts the binary ships.
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum Fragment {
            $($variant,)*
        }

        impl Fragment {
            pub const ALL: &[Fragment] = &[$(Fragment::$variant,)*];

            #[must_use]
            pub fn name(self) -> &'static str {
                match self {
                    $(Fragment::$variant => $name,)*
                }
            }

            /// The file `guide/shared/<name>.md`, verbatim.
            #[must_use]
            pub fn text(self) -> &'static str {
                match self {
                    $(Fragment::$variant => include_str!(concat!("../../guide/shared/", $name, ".md")),)*
                }
            }

            #[must_use]
            pub fn parse(name: &str) -> Option<Fragment> {
                Fragment::ALL.iter().copied().find(|f| f.name() == name)
            }
        }
    };
}

fragments! {
    Decisions "decisions",
    Questions "questions",
    Concise "concise",
    IdCitation "id-citation",
    AuthorStart "author-start",
    ArchitectStart "architect-start",
    ArchitectSpecEdits "architect-spec-edits",
    ArchitectOutput "architect-output",
    ArchitectEnd "architect-end",
    PlannerStart "planner-start",
    PlannerOutput "planner-output",
    PlannerRules "planner-rules",
    ReviewerStart "reviewer-start",
    ReviewerInputs "reviewer-inputs",
    ReviewerFollowUpStart "reviewer-follow-up-start",
    ReviewerFollowUpEnd "reviewer-follow-up-end",
    ReviewerOutput "reviewer-output",
}

/// The fragment an include line `<!-- include: <name> -->` names.
fn included(line: &str) -> Option<Fragment> {
    let name = line
        .trim()
        .strip_prefix("<!-- include:")?
        .strip_suffix("-->")?
        .trim();
    Fragment::parse(name)
}

/// `source` with every include line outside fenced code replaced by its
/// fragment's text. A line naming no fragment stays as it is.
pub(super) fn expand(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    for line in markdown::classified(source) {
        match included(line.text).filter(|_| !line.fenced) {
            Some(f) => out.push_str(f.text().trim_end_matches('\n')),
            None => out.push_str(line.text),
        }
        out.push('\n');
    }
    if !source.ends_with('\n') {
        out.pop();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// GA-R-050 — an include line becomes the fragment's text.
    #[test]
    fn ut_expand_splices_a_named_fragment() {
        let out = expand("a\n<!-- include: concise -->\nb\n");
        assert_eq!(
            out,
            format!("a\n{}\nb\n", Fragment::Concise.text().trim_end())
        );
        assert!(!expand("<!--include:concise-->").ends_with('\n'));
    }

    /// GA-R-050 — fenced and unknown include lines stay.
    #[test]
    fn ut_expand_keeps_fenced_and_unknown_lines() {
        let src = "```\n<!-- include: concise -->\n```\n<!-- include: nope -->\n";
        assert_eq!(expand(src), src);
    }

    #[test]
    fn ut_fragment_names_round_trip() {
        for f in Fragment::ALL {
            assert_eq!(Fragment::parse(f.name()), Some(*f));
            assert!(f.text().ends_with('\n') && !f.text().is_empty());
        }
        assert_eq!(Fragment::parse("nope"), None);
    }
}
