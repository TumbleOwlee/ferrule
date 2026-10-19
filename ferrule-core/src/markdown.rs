//! Markdown headings and sections, read line by line the way the workflow's
//! files are written: a heading is `#`s plus a space or tab at the start of a
//! line, and a line starting with three backticks opens or closes a fence
//! inside which nothing is a heading.

/// How a requested heading is compared with the file's headings.
///
/// A request is heading text, compared ignoring ASCII case: `Status` finds
/// a `Status` heading at any level. Leading `#`s and a space also pin the
/// level: `## Status` finds only level-2 ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Match {
    /// The heading text equals the request; every equal heading matches.
    Exact,
    /// The heading text starts with the request; only the first one matches,
    /// so `Stage s2:` finds `Stage s2: <title>` and never `Stage s20:`.
    Prefix,
}

/// A request's level, when it pins one, and its heading text.
fn request(want: &str) -> (Option<usize>, &str) {
    match heading_level(want) {
        Some(level) => (Some(level), want[level..].trim()),
        None => (None, want.trim()),
    }
}

fn hits(line: &str, level: usize, want: (Option<usize>, &str), how: Match) -> bool {
    if want.0.is_some_and(|l| l != level) {
        return false;
    }
    let text = line[level..].trim();
    let want = want.1;
    match how {
        Match::Exact => text.eq_ignore_ascii_case(want),
        Match::Prefix => text
            .get(..want.len())
            .is_some_and(|head| head.eq_ignore_ascii_case(want)),
    }
}

/// A heading line outside fenced code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Heading<'a> {
    pub level: usize,
    pub line: &'a str,
}

/// Splits text into lines the way `awk` reads them: a final newline does not
/// start another line.
pub fn lines(text: &str) -> impl Iterator<Item = &str> {
    let body = text.strip_suffix('\n').unwrap_or(text);
    (!text.is_empty())
        .then(|| body.split('\n'))
        .into_iter()
        .flatten()
}

fn heading_level(line: &str) -> Option<usize> {
    let level = line.bytes().take_while(|&b| b == b'#').count();
    match line.as_bytes().get(level) {
        Some(b' ' | b'\t') if level > 0 => Some(level),
        _ => None,
    }
}

/// Every line of `text`, paired with its heading level when it is a heading
/// outside fenced code.
fn classified(text: &str) -> impl Iterator<Item = (&str, Option<usize>)> {
    let mut fence = false;
    lines(text).map(move |line| {
        if line.starts_with("```") {
            fence = !fence;
        }
        let level = if fence { None } else { heading_level(line) };
        (line, level)
    })
}

/// The headings of `text` in file order, fenced code skipped.
#[must_use]
pub fn headings(text: &str) -> Vec<Heading<'_>> {
    classified(text)
        .filter_map(|(line, level)| level.map(|level| Heading { level, line }))
        .collect()
}

/// The lines of every section `want` names: each runs from its heading line
/// to the next heading of equal or shallower depth, or the end of the file.
/// Empty when no heading matches.
#[must_use]
pub fn section_lines<'a>(text: &'a str, want: &str, how: Match) -> Vec<&'a str> {
    let mut out = Vec::new();
    let mut printing = false;
    let mut start_level = 0;
    let mut done = false;
    let want = request(want);
    for (line, level) in classified(text) {
        if let Some(level) = level {
            if printing && level <= start_level {
                printing = false;
            }
            let hit = !(how == Match::Prefix && done) && hits(line, level, want, how);
            if hit {
                done = true;
                // A match nested in a section already printing stays part of it.
                if !printing {
                    start_level = level;
                }
                printing = true;
                out.push(line);
                continue;
            }
        }
        if printing {
            out.push(line);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOC: &str = "# Title\n\nIntro.\n\n## Alpha\nAlpha body.\n\n```sh\n# not a heading\n```\n\n### Alpha child\nChild body.\n\n## Beta\nBeta body.\n";

    #[test]
    fn ut_headings_skip_fenced_code() {
        let found: Vec<_> = headings(DOC).iter().map(|h| (h.level, h.line)).collect();
        assert_eq!(
            found,
            [
                (1, "# Title"),
                (2, "## Alpha"),
                (3, "### Alpha child"),
                (2, "## Beta")
            ]
        );
    }

    #[test]
    fn ut_heading_needs_space_or_tab() {
        assert_eq!(heading_level("##Alpha"), None);
        assert_eq!(heading_level("##\tAlpha"), Some(2));
        assert_eq!(heading_level("#"), None);
    }

    #[test]
    fn ut_section_ends_at_equal_or_shallower_heading() {
        let got = section_lines(DOC, "## Alpha", Match::Exact);
        assert_eq!(got.first(), Some(&"## Alpha"));
        assert_eq!(got.last(), Some(&""));
        assert!(got.contains(&"### Alpha child"));
        assert!(!got.contains(&"## Beta"));
    }

    #[test]
    fn ut_prefix_takes_only_the_first_match() {
        let text = "## Stage s2: a\none\n## Stage s2: b\ntwo\n";
        assert_eq!(
            section_lines(text, "## Stage s2:", Match::Prefix),
            ["## Stage s2: a", "one"]
        );
        assert_eq!(
            section_lines(text, "## Stage s2: b", Match::Exact),
            ["## Stage s2: b", "two"]
        );
    }

    #[test]
    fn ut_request_without_hashes_matches_any_level() {
        let text = "# Status\ntop\n## Status\nsub\n## Next\n";
        assert_eq!(
            section_lines(text, "Status", Match::Exact),
            ["# Status", "top", "## Status", "sub", "## Next"]
        );
        assert_eq!(
            section_lines(text, "## Status", Match::Exact),
            ["## Status", "sub"]
        );
        assert_eq!(
            section_lines(text, "status", Match::Exact),
            section_lines(text, "Status", Match::Exact)
        );
        assert_eq!(
            section_lines(DOC, "alpha ch", Match::Prefix),
            ["### Alpha child", "Child body.", ""]
        );
        assert_eq!(section_lines(DOC, "Alph", Match::Exact), Vec::<&str>::new());
    }

    #[test]
    fn ut_lines_of_empty_text() {
        assert_eq!(lines("").count(), 0);
        assert_eq!(lines("a\n\n").collect::<Vec<_>>(), ["a", ""]);
        assert_eq!(lines("a").collect::<Vec<_>>(), ["a"]);
    }
}
