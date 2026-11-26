//! Syntax trees of test files: a file's grammar by extension and the last
//! line of a test's body by tree. The only module naming tree-sitter types.

use std::ops::Range;

/// A language the body of a test is found in by syntax tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Grammar {
    Rust,
    Python,
    JavaScript,
    TypeScript,
    Tsx,
    Go,
}

impl Grammar {
    /// By extension: rs, py, js/mjs/cjs/jsx, ts/mts/cts, tsx, go.
    pub(super) fn for_path(path: &str) -> Option<Grammar> {
        let file = path.rsplit('/').next()?;
        let (_, extension) = file.rsplit_once('.')?;
        match extension {
            "rs" => Some(Grammar::Rust),
            "py" => Some(Grammar::Python),
            "js" | "mjs" | "cjs" | "jsx" => Some(Grammar::JavaScript),
            "ts" | "mts" | "cts" => Some(Grammar::TypeScript),
            "tsx" => Some(Grammar::Tsx),
            "go" => Some(Grammar::Go),
            _ => None,
        }
    }

    fn language(self) -> tree_sitter::Language {
        match self {
            Grammar::Rust => tree_sitter_rust::LANGUAGE.into(),
            Grammar::Python => tree_sitter_python::LANGUAGE.into(),
            Grammar::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
            Grammar::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Grammar::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
            Grammar::Go => tree_sitter_go::LANGUAGE.into(),
        }
    }
}

/// A parsed test file.
pub(super) struct Parsed {
    tree: tree_sitter::Tree,
}

impl Parsed {
    /// Always a tree: no timeout or cancellation is set, so tree-sitter
    /// cannot return none; a broken side shows as error nodes.
    pub(super) fn parse(grammar: Grammar, text: &str) -> Parsed {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&grammar.language())
            .expect("a linked grammar loads");
        let tree = parser
            .parse(text, None)
            .expect("parsing without timeout or cancellation yields a tree");
        Parsed { tree }
    }

    /// Last line (0-based) of the body of the test whose name spans the bytes
    /// `name` and whose match covers lines `first..=last`: the innermost node
    /// holding `name`, root excluded, that ends after `last`, when it starts
    /// at or after `first` (GW-R-114); `last` when it starts before or no such
    /// node exists (GW-R-114); `None` when that node `has_error()` (GW-R-116).
    pub(super) fn body_end(&self, name: Range<usize>, first: usize, last: usize) -> Option<usize> {
        let root = self.tree.root_node();
        let found = root
            .descendant_for_byte_range(name.start, name.end)
            .into_iter()
            .flat_map(|node| std::iter::successors(Some(node), tree_sitter::Node::parent))
            .take_while(|node| node.id() != root.id())
            .find(|node| end_line(*node) > last);
        match found {
            Some(node) if node.has_error() => None,
            Some(node) if node.start_position().row >= first => Some(end_line(node)),
            _ => Some(last),
        }
    }
}

/// Last line of a node; a node closed by its trailing newline ends on the
/// line before.
fn end_line(node: tree_sitter::Node<'_>) -> usize {
    let end = node.end_position();
    if end.column == 0 && end.row > node.start_position().row {
        end.row - 1
    } else {
        end.row
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(text: &str, needle: &str) -> Range<usize> {
        let at = text.find(needle).expect("needle in text");
        at..at + needle.len()
    }

    /// GW-R-113 — a file's language follows its extension, case-sensitively,
    /// by the path's last component.
    #[test]
    fn ut_syntax_language_follows_extension() {
        for (path, want) in [
            ("a.rs", Grammar::Rust),
            ("a.py", Grammar::Python),
            ("a.js", Grammar::JavaScript),
            ("a.mjs", Grammar::JavaScript),
            ("a.cjs", Grammar::JavaScript),
            ("a.jsx", Grammar::JavaScript),
            ("a.ts", Grammar::TypeScript),
            ("a.mts", Grammar::TypeScript),
            ("a.cts", Grammar::TypeScript),
            ("a.tsx", Grammar::Tsx),
            ("a.go", Grammar::Go),
            ("dir/sub.d/a.rs", Grammar::Rust),
        ] {
            assert_eq!(Grammar::for_path(path), Some(want), "{path}");
        }
        for path in ["src/lib.kt", "Makefile", "a.RS", "dir.rs/file", ""] {
            assert_eq!(Grammar::for_path(path), None, "{path}");
        }
    }

    /// GW-R-113 — every supported grammar loads and parses a valid file.
    #[test]
    fn ut_syntax_every_grammar_parses() {
        for (grammar, text) in [
            (Grammar::Rust, "#[test]\nfn t_a() {\n    a();\n}\n"),
            (Grammar::Python, "def test_a():\n    a()\n"),
            (Grammar::JavaScript, "test(\"a\", () => {\n  a();\n});\n"),
            (
                Grammar::TypeScript,
                "test(\"a\", (): void => {\n  a();\n});\n",
            ),
            (
                Grammar::Tsx,
                "test(\"a\", () => {\n  const x = <b/>;\n});\n",
            ),
            (
                Grammar::Go,
                "package p\n\nfunc TestA(t *testing.T) {\n\ta()\n}\n",
            ),
        ] {
            let parsed = Parsed::parse(grammar, text);
            assert!(!parsed.tree.root_node().has_error(), "{grammar:?}");
        }
    }

    /// GW-R-114 — the body ends at the last line of the innermost node holding
    /// the name that starts at or after the match and ends after it.
    #[test]
    fn ut_syntax_body_end_is_the_node_end() {
        let rust = "#[test]\nfn t_a() {\n    a();\n}\n";
        let parsed = Parsed::parse(Grammar::Rust, rust);
        assert_eq!(parsed.body_end(span(rust, "t_a"), 0, 1), Some(3));

        let py = "@dec\ndef test_a():\n    a()\n    b()\n\nx = 1\n";
        let parsed = Parsed::parse(Grammar::Python, py);
        assert_eq!(parsed.body_end(span(py, "test_a"), 0, 1), Some(3));
    }

    /// GW-R-114 — with no body node starting inside the match, the body ends
    /// at the match's last line.
    #[test]
    fn ut_syntax_body_end_without_node_is_the_match_end() {
        let js = "test(\"a\", () => x());\nfoo();\n";
        let parsed = Parsed::parse(Grammar::JavaScript, js);
        assert_eq!(parsed.body_end(span(js, "\"a\""), 0, 0), Some(0));

        let rust = "mod tests {\n    fn t_a() {}\n}\n";
        let parsed = Parsed::parse(Grammar::Rust, rust);
        assert_eq!(parsed.body_end(span(rust, "t_a"), 1, 1), Some(1));
    }

    /// GW-R-116 — a body node holding a parse error yields no body end, even
    /// where the no-node branch would otherwise apply.
    #[test]
    fn ut_syntax_body_end_none_on_parse_error() {
        let rust = "fn t_a() {\n    let x = ;\n}\n";
        let parsed = Parsed::parse(Grammar::Rust, rust);
        assert_eq!(parsed.body_end(span(rust, "t_a"), 0, 0), None);

        let rust = "mod tests {\n    fn t_a() {}\n    fn t_b() { let x = ; }\n}\n";
        let parsed = Parsed::parse(Grammar::Rust, rust);
        assert_eq!(parsed.body_end(span(rust, "t_a"), 1, 1), None);
    }
}
