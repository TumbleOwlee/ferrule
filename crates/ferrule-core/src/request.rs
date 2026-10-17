//! A reused issue's original request, read back from its body: the text
//! the user wrote, never a body this workflow assembled around it.

/// Which part of the issue to print.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    /// The request as the user filed it.
    Request,
    /// An assembled body's spec, architecture, requirements and scenarios.
    Spec,
    /// The epic number of a slice issue.
    Epic,
    /// A slice issue's whole `**Part of epic**` line.
    SliceLine,
}

/// Whether this workflow assembled the body (or an older one did: those
/// start `**Status** `).
fn assembled(body: &str) -> bool {
    body.contains("## Spec (rev")
        || body.contains("## Scenarios")
        || body.starts_with("**Status** ")
        || body.contains("<summary>Original request</summary>")
}

fn drop_first_blank(mut lines: Vec<&str>) -> Vec<&str> {
    if lines.first() == Some(&"") {
        lines.remove(0);
    }
    lines
}

/// The issue body with its surrounding blank lines dropped.
fn trimmed(body: &str) -> Vec<&str> {
    let mut lines: Vec<&str> = body.lines().collect();
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    drop_first_blank(lines)
}

const EPIC: &str = "**Part of epic** #";

/// The lines of `part`; none when the body holds no such part.
#[must_use]
pub fn part(body: &str, part: Part) -> Vec<String> {
    let lines = trimmed(body);
    let owned = |v: Vec<&str>| v.into_iter().map(str::to_owned).collect();
    match part {
        Part::SliceLine => owned(
            lines
                .into_iter()
                .filter(|l| l.starts_with(EPIC))
                .take(1)
                .collect(),
        ),
        Part::Epic => lines
            .iter()
            .filter_map(|l| l.strip_prefix(EPIC))
            .map(|r| {
                r.chars()
                    .take_while(char::is_ascii_digit)
                    .collect::<String>()
            })
            .find(|n| !n.is_empty())
            .into_iter()
            .collect(),
        Part::Spec => {
            if !assembled(&lines.join("\n")) {
                return Vec::new();
            }
            let mut on = false;
            let mut out = Vec::new();
            for l in lines {
                if l.starts_with("## Spec (rev")
                    || ["## Architecture", "## Requirements", "## Scenarios"].contains(&l)
                {
                    on = true;
                } else if l.starts_with("## ") || l.starts_with("<details>") {
                    on = false;
                }
                if on {
                    out.push(l.to_owned());
                }
            }
            out
        }
        Part::Request => {
            if !assembled(&lines.join("\n")) {
                return owned(lines);
            }
            let mut on = false;
            let mut out = Vec::new();
            for l in lines {
                if l.starts_with("</details>") {
                    break;
                }
                if on {
                    out.push(l);
                }
                if l.contains("<summary>") {
                    on = true;
                }
            }
            owned(drop_first_blank(out))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ut_request_inside_details_only_when_assembled() {
        let body = "## Goal\nx\n\n<details><summary>Original request</summary>\n\nplease retry\n</details>\n";
        assert_eq!(part(body, Part::Request), ["please retry"]);
        assert_eq!(part("\nplain\n\n", Part::Request), ["plain"]);
    }

    #[test]
    fn ut_epic_and_slice_line() {
        let body = "**Part of epic** #70 · **Slice** 1 of 3\n";
        assert_eq!(part(body, Part::Epic), ["70"]);
        assert_eq!(
            part(body, Part::SliceLine),
            ["**Part of epic** #70 · **Slice** 1 of 3"]
        );
        assert!(part("x\n", Part::Epic).is_empty());
    }
}
