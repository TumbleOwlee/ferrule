//! A topic's `##` sections: selecting some by heading, and a project's
//! override merged over the built-in text section by section.

use crate::markdown;

/// The section spans of `text`: (heading line, start byte, end byte), each
/// through the next heading of equal or shallower depth.
pub(super) fn sections(text: &str) -> Vec<(String, usize, usize)> {
    // Each heading is a slice of `text`: its offset is where it starts.
    let starts: Vec<(usize, usize, String)> = markdown::headings(text)
        .iter()
        .map(|h| {
            let start = h.line.as_ptr() as usize - text.as_ptr() as usize;
            (start, h.level, h.line.trim_end().to_owned())
        })
        .collect();
    starts
        .iter()
        .enumerate()
        .map(|(i, (start, level, heading))| {
            let end = starts[i + 1..]
                .iter()
                .find(|(_, l, _)| l <= level)
                .map_or(text.len(), |(s, _, _)| *s);
            (heading.clone(), *start, end)
        })
        .collect()
}

/// A heading's text without its `#`s and without the marks `render` adds,
/// `(fast lane)`, `(full lane)`, `(done)`.
fn heading_text(line: &str) -> &str {
    let text = line.trim_start_matches('#').trim();
    if let Some((bare, mark)) = text.rsplit_once(" (")
        && let Some(mark) = mark.strip_suffix(')')
        && mark
            .split(", ")
            .all(|m| matches!(m, "fast lane" | "full lane" | "done"))
    {
        return bare;
    }
    text
}

/// The topic without its `# <name> — <summary>` line, which only `list`
/// shows.
#[must_use]
pub fn body(text: &str) -> &str {
    if !text.starts_with("# ") {
        return text;
    }
    let rest = text.split_once('\n').map_or("", |(_, r)| r);
    rest.trim_start_matches('\n')
}

/// The text of every heading in `text`, in order.
#[must_use]
pub fn headings(text: &str) -> Vec<String> {
    sections(text)
        .into_iter()
        .map(|(h, _, _)| heading_text(&h).to_owned())
        .collect()
}

/// The sections whose heading text equals one of `wanted`, ignoring case,
/// in the order asked; `Err` names the headings `text` lacks.
pub fn select(text: &str, wanted: &[String]) -> Result<String, Vec<String>> {
    let all = sections(text);
    let mut out = String::new();
    let mut missing = Vec::new();
    for w in wanted {
        match all
            .iter()
            .find(|(h, _, _)| heading_text(h).eq_ignore_ascii_case(heading_text(w)))
        {
            Some((_, start, end)) => {
                if !out.is_empty() && !out.ends_with("\n\n") {
                    out.push('\n');
                }
                out.push_str(&text[*start..*end]);
            }
            None => missing.push(w.clone()),
        }
    }
    if missing.is_empty() {
        Ok(out)
    } else {
        Err(missing)
    }
}

/// `default` with `over` laid on top (module docs).
#[must_use]
pub fn merge(default: &str, over: &str) -> String {
    let over_sections = sections(over);
    if over_sections.is_empty() {
        return over.to_owned();
    }
    let mut out = default.to_owned();
    // Top-level override sections only; their subsections travel with them.
    let mut covered = 0;
    for (heading, start, end) in &over_sections {
        if *start < covered {
            continue;
        }
        covered = *end;
        let mut body = over[*start..*end].to_owned();
        if !body.ends_with('\n') {
            body.push('\n');
        }
        if let Some((_, s, e)) = sections(&out).into_iter().find(|(h, _, _)| h == heading) {
            if out[s..e].ends_with("\n\n") && !body.ends_with("\n\n") {
                body.push('\n');
            }
            out.replace_range(s..e, &body);
        } else {
            if !out.ends_with("\n\n") {
                out.push_str(if out.ends_with('\n') { "\n" } else { "\n\n" });
            }
            out.push_str(&body);
        }
    }
    out
}
