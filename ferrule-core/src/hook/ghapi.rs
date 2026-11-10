//! `gh api` calls, parsed far enough to tell a read from a write and to
//! name the repository a write spells out.

/// What a `gh api` call does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ApiCall<'a> {
    /// Changes nothing.
    Read,
    /// Writes; `repo` is the `owner/name` its endpoint spells out, `None`
    /// when no repository can be proven and for every graphql write.
    Write { repo: Option<&'a str> },
}

/// The options of a `gh api` call that matter, and whether any word was
/// one the parse does not know.
struct Parsed<'a> {
    endpoint: Option<&'a str>,
    /// A method other than GET, a field or an input.
    writes: bool,
    /// The `--input` file.
    input: Option<&'a str>,
    hostname: bool,
    /// The values of the fields, `key=value` as given.
    fields: Vec<&'a str>,
    unknown: bool,
}

/// Long options that take a value, and the booleans.
const LONG_VALUED: &[&str] = &[
    "--method",
    "--field",
    "--raw-field",
    "--header",
    "--input",
    "--jq",
    "--template",
    "--cache",
    "--preview",
    "--hostname",
];
const LONG_BOOL: &[&str] = &[
    "--paginate",
    "--silent",
    "--include",
    "--verbose",
    "--slurp",
];

fn not_get(method: &str) -> bool {
    !method.trim_start_matches('=').eq_ignore_ascii_case("GET")
}

/// Parses the words after `gh`; `None` when they do not start at `api`.
fn parse(args: &[String]) -> Option<Parsed<'_>> {
    if args.first().map(String::as_str) != Some("api") {
        return None;
    }
    let mut p = Parsed {
        endpoint: None,
        writes: false,
        input: None,
        hostname: false,
        fields: Vec::new(),
        unknown: false,
    };
    let mut i = 1;
    while let Some(w) = args.get(i) {
        i += 1;
        if let Some(long) = w.strip_prefix("--") {
            let (name, value) = match long.split_once('=') {
                Some((n, v)) => (format!("--{n}"), Some(v)),
                None => (format!("--{long}"), None),
            };
            if LONG_BOOL.contains(&name.as_str()) && value.is_none() {
                continue;
            }
            if !LONG_VALUED.contains(&name.as_str()) {
                p.unknown = true;
                continue;
            }
            let value = value.or_else(|| {
                let v = args.get(i).map(String::as_str);
                i += 1;
                v
            });
            let Some(value) = value else {
                p.unknown = true;
                continue;
            };
            p.apply(&name, value);
        } else if w.len() > 1 && w.starts_with('-') {
            let cluster = &w[1..];
            for (at, ch) in cluster.char_indices() {
                if ch == 'i' {
                    continue;
                }
                let name = match ch {
                    'X' => "--method",
                    'f' => "--raw-field",
                    'F' => "--field",
                    'H' => "--header",
                    'q' => "--jq",
                    't' => "--template",
                    'p' => "--preview",
                    _ => {
                        p.unknown = true;
                        break;
                    }
                };
                let rest = &cluster[at + 1..];
                let value = if rest.is_empty() {
                    let v = args.get(i).map(String::as_str);
                    i += 1;
                    v
                } else {
                    Some(rest)
                };
                match value {
                    Some(v) => p.apply(name, v),
                    None => p.unknown = true,
                }
                break;
            }
        } else if p.endpoint.is_none() {
            p.endpoint = Some(w);
        }
    }
    Some(p)
}

impl<'a> Parsed<'a> {
    fn apply(&mut self, name: &str, value: &'a str) {
        match name {
            "--method" => self.writes |= not_get(value),
            "--field" | "--raw-field" => {
                self.writes = true;
                self.fields.push(value);
            }
            "--input" => {
                self.writes = true;
                self.input = Some(value);
            }
            "--hostname" => self.hostname = true,
            _ => {}
        }
    }
}

/// The `owner/name` an endpoint spells out after `repos/`, or `None`.
fn repo_of(endpoint: &str) -> Option<&str> {
    if endpoint.contains("..") || endpoint.contains(['%', '$', '`']) {
        return None;
    }
    let mut parts = endpoint.split('/');
    if parts.next() != Some("repos") {
        return None;
    }
    let (owner, name) = (parts.next()?, parts.next()?);
    let ok = |s: &str| {
        !s.is_empty()
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    };
    (ok(owner) && ok(name)).then(|| &endpoint["repos/".len()..][..owner.len() + 1 + name.len()])
}

/// `args` are the words after `gh`, starting at `api`.
pub(super) fn classify(args: &[String]) -> ApiCall<'_> {
    let Some(p) = parse(args) else {
        return ApiCall::Write { repo: None };
    };
    if p.unknown {
        return ApiCall::Write { repo: None };
    }
    if p.endpoint == Some("graphql") {
        return if graphql_read(args) {
            ApiCall::Read
        } else {
            ApiCall::Write { repo: None }
        };
    }
    if !p.writes {
        return ApiCall::Read;
    }
    let repo = if p.hostname {
        None
    } else {
        p.endpoint.and_then(repo_of)
    };
    ApiCall::Write { repo }
}

/// A `gh api graphql` call that only reads.
pub(super) fn graphql_read(args: &[String]) -> bool {
    let Some(p) = parse(args) else {
        return false;
    };
    if p.unknown || p.input.is_some() || p.endpoint != Some("graphql") {
        return false;
    }
    let queries: Vec<&str> = p
        .fields
        .iter()
        .filter_map(|f| f.split_once('=').filter(|(k, _)| *k == "query"))
        .map(|(_, v)| v)
        .collect();
    // A `query` key with no value is no query.
    let bare = p.fields.contains(&"query");
    !bare && !queries.is_empty() && queries.iter().all(|q| only_queries(q))
}

/// Whether a graphql document holds nothing but `query` and `fragment`
/// definitions and anonymous `{` operations at its top level.
fn only_queries(q: &str) -> bool {
    if q.starts_with('@') {
        return false;
    }
    if expands_outside_parens(q) {
        return false;
    }
    let text = strip(q);
    let (mut brace, mut paren) = (0usize, 0usize);
    let (mut first, mut defs) = (None::<String>, 0usize);
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '(' => paren += 1,
            ')' => {
                let Some(n) = paren.checked_sub(1) else {
                    return false;
                };
                paren = n;
            }
            _ if paren > 0 => {}
            '{' => {
                if brace == 0 {
                    defs += 1;
                    if first
                        .as_deref()
                        .is_some_and(|w| !matches!(w, "query" | "fragment"))
                    {
                        return false;
                    }
                    first = None;
                }
                brace += 1;
            }
            '}' => {
                let Some(n) = brace.checked_sub(1) else {
                    return false;
                };
                brace = n;
            }
            c if brace == 0 && (c.is_ascii_alphanumeric() || c == '_') => {
                let start = i;
                while i + 1 < chars.len()
                    && (chars[i + 1].is_ascii_alphanumeric() || chars[i + 1] == '_')
                {
                    i += 1;
                }
                if first.is_none() {
                    first = Some(chars[start..=i].iter().collect());
                }
            }
            _ => {}
        }
        i += 1;
    }
    brace == 0 && paren == 0 && defs > 0 && first.is_none()
}

/// Whether a `$` or backtick sits outside every pair of parentheses,
/// strings and comments included. Parentheses inside a string or comment
/// do not count.
fn expands_outside_parens(q: &str) -> bool {
    let c: Vec<char> = q.chars().collect();
    let (mut depth, mut i) = (0usize, 0);
    while i < c.len() {
        let start = i;
        match c[i] {
            '#' => {
                while i < c.len() && c[i] != '\n' {
                    i += 1;
                }
            }
            '"' if c[i..].starts_with(&['"', '"', '"']) => {
                i += 3;
                while i < c.len() && !c[i..].starts_with(&['"', '"', '"']) {
                    i += 1;
                }
                i += 3;
            }
            '"' => {
                i += 1;
                while i < c.len() && c[i] != '"' {
                    i += if c[i] == '\\' { 2 } else { 1 };
                }
                i += 1;
            }
            '(' => {
                depth += 1;
                i += 1;
            }
            ')' => {
                depth = depth.saturating_sub(1);
                i += 1;
            }
            _ => i += 1,
        }
        if depth == 0
            && c[start..i.min(c.len())]
                .iter()
                .any(|ch| matches!(ch, '$' | '`'))
        {
            return true;
        }
    }
    false
}

/// A document without its `#` comments and string literals.
fn strip(q: &str) -> String {
    let c: Vec<char> = q.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < c.len() {
        match c[i] {
            '#' => {
                while i < c.len() && c[i] != '\n' {
                    i += 1;
                }
            }
            '"' if c[i..].starts_with(&['"', '"', '"']) => {
                i += 3;
                while i < c.len() && !c[i..].starts_with(&['"', '"', '"']) {
                    i += 1;
                }
                i += 3;
            }
            '"' => {
                i += 1;
                while i < c.len() && c[i] != '"' {
                    i += if c[i] == '\\' { 2 } else { 1 };
                }
                i += 1;
            }
            ch => {
                out.push(ch);
                i += 1;
            }
        }
    }
    out
}

/// `--help` is the last word, follows no option and no word is an
/// expansion.
pub(super) fn help(args: &[String]) -> bool {
    args.last().is_some_and(|w| w == "--help")
        && !args[..args.len() - 1].iter().any(|w| w.starts_with('-'))
        && !args.iter().any(|w| w.contains(['$', '`']))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(line: &str) -> Vec<String> {
        line.split(' ').map(str::to_owned).collect()
    }

    fn call(line: &str) -> ApiCall<'static> {
        let w: &'static Vec<String> = Box::leak(Box::new(words(line)));
        classify(w)
    }

    fn is_write(line: &str) -> bool {
        matches!(call(line), ApiCall::Write { .. })
    }

    /// The write forms stay writes and a plain GET reads.
    #[test]
    fn ut_classify_reads_and_writes() {
        for w in [
            "api repos/o/r -f a=b",
            "api repos/o/r -XPOST",
            "api repos/o/r --method=PUT",
            "api repos/o/r --input=x",
            "api repos/o/r -iXPOST",
            "api repos/o/r -if x=1",
        ] {
            assert!(is_write(w), "{w}");
        }
        for w in ["api repos/o/r", "api -i repos/o/r", "api -X GET repos/o/r"] {
            assert_eq!(call(w), ApiCall::Read, "{w}");
        }
        for w in [
            "--hostname h api repos/o/r",
            "api --bogus repos/o/r",
            "api -z repos/o/r",
        ] {
            assert_eq!(call(w), ApiCall::Write { repo: None }, "{w}");
        }
    }

    /// SG-E-015 — a write names the repository its endpoint spells out.
    #[test]
    fn ut_classify_names_the_spelled_out_repository() {
        assert_eq!(
            call("api -X POST repos/o/r/issues"),
            ApiCall::Write { repo: Some("o/r") }
        );
        for w in [
            "api -X POST {owner}/x",
            "api -X POST /repos/o/r",
            "api -X POST https://api.github.com/repos/o/r",
        ] {
            assert_eq!(call(w), ApiCall::Write { repo: None }, "{w}");
        }
    }

    /// SG-E-016 — nothing that could move the call names a repository.
    #[test]
    fn ut_classify_refuses_what_could_move_the_call() {
        for w in [
            "api -X POST --hostname h repos/o/r",
            "api -X POST repos/o/r/../x",
            "api -X POST repos/o%2fx/r",
            "api -X POST repos/o/r/$X",
            "api -X POST repos/o/r/`x`",
        ] {
            assert_eq!(call(w), ApiCall::Write { repo: None }, "{w}");
        }
        assert_eq!(
            call("api -X POST repos/o/r-evil"),
            ApiCall::Write {
                repo: Some("o/r-evil")
            }
        );
    }

    fn gq(line: &str) -> bool {
        graphql_read(&words(line))
    }

    /// SG-R-025, SG-E-009, SG-E-018, SG-E-019 — a graphql call reads only
    /// when every query field holds queries and nothing else.
    #[test]
    fn ut_graphql_read_takes_queries_only() {
        for w in [
            "api graphql -f query=query{a}",
            "api graphql -f query={viewer{login}}",
            "api graphql -fquery=query{a}",
            "api graphql -f query=query($n:Int){a(n:$n)} -F n=1",
        ] {
            assert!(gq(w), "{w}");
        }
        for w in [
            "api graphql -f query=mutation{a}",
            "api graphql -f query=query{a}mutation{b}",
            "api graphql -f query=@f",
            "api graphql --input x",
            "api graphql -f query=$Q",
            "api graphql -F id=x",
            "api graphql -f query=query{a} -if query=mutation{b}",
            "api graphql --bogus -f query=query{a}",
            "--hostname h api graphql -f query=query{a}",
        ] {
            assert!(!gq(w), "{w}");
        }
        let multi = vec![
            "api".to_owned(),
            "graphql".to_owned(),
            "-f".to_owned(),
            "query=fragment F on X { a }\nquery Q { ...F } # mutation".to_owned(),
        ];
        assert!(graphql_read(&multi));
        let hidden = vec![
            "api".to_owned(),
            "graphql".to_owned(),
            "-f".to_owned(),
            "query=query { a(b: \"mutation {\") }\n# }\nmutation { c }".to_owned(),
        ];
        assert!(!graphql_read(&hidden));
        let paren_in_string = vec![
            "api".to_owned(),
            "graphql".to_owned(),
            "-f".to_owned(),
            "query=query { a(b: \"(\") } $query { c }".to_owned(),
        ];
        assert!(!graphql_read(&paren_in_string));
        let paren_in_comment = vec![
            "api".to_owned(),
            "graphql".to_owned(),
            "-f".to_owned(),
            "query=query { a } # (\n$query { c }".to_owned(),
        ];
        assert!(!graphql_read(&paren_in_comment));
    }

    /// SG-E-017 — `--help` opens a call only as its plain last word.
    #[test]
    fn ut_help_needs_a_plain_last_word() {
        assert!(help(&words("pr list --help")));
        for w in [
            "pr list --help x",
            "issue create --title --help",
            "pr list --state open --help",
            "$X list --help",
        ] {
            assert!(!help(&words(w)), "{w}");
        }
    }
}
