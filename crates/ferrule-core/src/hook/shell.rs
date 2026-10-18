//! Enough of the POSIX shell grammar to see what a Bash tool call runs: its
//! simple commands with their words after quote removal, their redirections,
//! heredoc bodies, and which output feeds a pipe. Commands nested in `$(…)`,
//! backticks and `<(…)` are listed after the command holding them. No
//! expansion beyond quote removal: `$x` stays `$x`.

use std::ops::Range;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RedirectKind {
    /// `>`, `>|`, `&>`, `<>`.
    Write,
    /// `>>`, `&>>`.
    Append,
    /// `<`.
    Read,
    /// `<<`, `<<-`: the target is the delimiter, the body is in `body`.
    HereDoc,
    /// `<<<`.
    HereString,
    /// `>&2`, `2>&1`, `<&0`, `>&-`.
    Dup,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Redirect {
    pub kind: RedirectKind,
    pub target: String,
    /// A heredoc's body, every line ending in `\n`.
    pub body: Option<String>,
    /// Where the heredoc's body sits in the source, in chars.
    pub body_span: Option<Range<usize>>,
    heredoc: Option<usize>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Command {
    pub words: Vec<String>,
    /// Where each word sits in the source, in chars; empty for a command
    /// inside backticks, whose source is the unescaped substitution.
    pub spans: Vec<Range<usize>>,
    pub redirects: Vec<Redirect>,
    /// Its stdout feeds the next command through `|` or `|&`.
    pub piped: bool,
    /// It runs inside `$(…)`, backticks or `<(…)`, its output captured.
    pub substituted: bool,
}

impl Command {
    /// Its output goes somewhere other than the terminal: a pipe or a file.
    #[must_use]
    pub fn output_captured(&self) -> bool {
        self.piped
            || self.substituted
            || self
                .redirects
                .iter()
                .any(|r| matches!(r.kind, RedirectKind::Write | RedirectKind::Append))
    }
}

/// The simple commands of `src`, in order.
#[must_use]
pub fn parse(src: &str) -> Vec<Command> {
    let mut lexer = Lexer {
        chars: src.chars().collect(),
        pos: 0,
        bodies: Vec::new(),
        pending: Vec::new(),
    };
    let mut commands = lexer.list(false);
    for command in &mut commands {
        for r in &mut command.redirects {
            if let Some(Some((body, span))) = r.heredoc.and_then(|id| lexer.bodies.get(id)) {
                r.body = Some(body.clone());
                r.body_span = Some(span.clone());
            }
        }
    }
    commands
}

struct Lexer {
    chars: Vec<char>,
    pos: usize,
    bodies: Vec<Option<(String, Range<usize>)>>,
    /// Heredocs whose bodies start after the next newline: id, delimiter,
    /// whether leading tabs are stripped (`<<-`).
    pending: Vec<(usize, String, bool)>,
}

fn ends_word(c: char) -> bool {
    matches!(
        c,
        ' ' | '\t' | '\n' | ';' | '&' | '|' | '<' | '>' | '(' | ')'
    )
}

impl Lexer {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn peek_at(&self, n: usize) -> Option<char> {
        self.chars.get(self.pos + n).copied()
    }

    fn eat(&mut self, s: &str) -> bool {
        let n = s.chars().count();
        let matches = self.chars.len() >= self.pos + n
            && self.chars[self.pos..self.pos + n]
                .iter()
                .copied()
                .eq(s.chars());
        if matches {
            self.pos += n;
        }
        matches
    }

    /// Commands up to the end, or up to the `)` closing a nested list.
    fn list(&mut self, nested: bool) -> Vec<Command> {
        let mut out = Vec::new();
        let mut inner = Vec::new();
        let mut cur = Command::default();
        let mut word_start = true;
        let finish = |cur: &mut Command, out: &mut Vec<Command>, inner: &mut Vec<Command>| {
            if !cur.words.is_empty() || !cur.redirects.is_empty() {
                out.push(std::mem::take(cur));
            }
            *cur = Command::default();
            out.append(inner);
        };
        while let Some(c) = self.peek() {
            match c {
                ' ' | '\t' => {
                    self.pos += 1;
                    word_start = true;
                    continue;
                }
                '\\' if self.peek_at(1) == Some('\n') => {
                    self.pos += 2;
                    word_start = true;
                    continue;
                }
                '#' if word_start => {
                    while self.peek().is_some_and(|c| c != '\n') {
                        self.pos += 1;
                    }
                    continue;
                }
                ')' if nested => {
                    self.pos += 1;
                    break;
                }
                '\n' => {
                    self.pos += 1;
                    finish(&mut cur, &mut out, &mut inner);
                    self.heredoc_bodies();
                }
                '&' if matches!(self.peek_at(1), Some('>')) => {
                    self.pos += 1;
                    self.redirect(&mut cur, &mut inner);
                }
                ';' | '&' | '|' => {
                    if self.eat("|&") || (self.peek_at(1) != Some('|') && self.eat("|")) {
                        cur.piped = true;
                    } else if !self.eat("&&") && !self.eat("||") && !self.eat(";;") {
                        self.pos += 1;
                    }
                    finish(&mut cur, &mut out, &mut inner);
                }
                '<' | '>' if self.peek_at(1) == Some('(') => {
                    self.pos += 2;
                    inner.extend(substituted(self.list(true)));
                }
                '<' | '>' => self.redirect(&mut cur, &mut inner),
                '(' | ')' => {
                    self.pos += 1;
                    finish(&mut cur, &mut out, &mut inner);
                }
                d if d.is_ascii_digit() && word_start && self.fd_redirect() => {
                    self.redirect(&mut cur, &mut inner);
                }
                _ => {
                    let start = self.pos;
                    let word = self.word(&mut inner);
                    cur.words.push(word);
                    cur.spans.push(start..self.pos);
                }
            }
            word_start = true;
        }
        finish(&mut cur, &mut out, &mut inner);
        out
    }

    /// At digits followed by `<` or `>`: skips the digits.
    fn fd_redirect(&mut self) -> bool {
        let digits = self.chars[self.pos..]
            .iter()
            .take_while(|c| c.is_ascii_digit())
            .count();
        let is = matches!(self.peek_at(digits), Some('<' | '>'));
        if is {
            self.pos += digits;
        }
        is
    }

    fn redirect(&mut self, cur: &mut Command, inner: &mut Vec<Command>) {
        let kind = if self.eat("<<<") {
            RedirectKind::HereString
        } else if self.eat("<<-") {
            return self.heredoc(cur, true);
        } else if self.eat("<<") {
            return self.heredoc(cur, false);
        } else if self.eat(">>") {
            RedirectKind::Append
        } else if self.eat(">&") || self.eat("<&") {
            RedirectKind::Dup
        } else if self.eat(">|") || self.eat("<>") || self.eat(">") {
            RedirectKind::Write
        } else {
            self.pos += 1;
            RedirectKind::Read
        };
        self.blanks();
        let target = self.word(inner);
        let kind = match kind {
            RedirectKind::Dup if !target.chars().all(|c| c.is_ascii_digit() || c == '-') => {
                RedirectKind::Write
            }
            k => k,
        };
        cur.redirects.push(Redirect {
            kind,
            target,
            body: None,
            body_span: None,
            heredoc: None,
        });
    }

    fn heredoc(&mut self, cur: &mut Command, strip_tabs: bool) {
        self.blanks();
        let delimiter = self.word(&mut Vec::new());
        let id = self.bodies.len();
        self.bodies.push(None);
        self.pending.push((id, delimiter.clone(), strip_tabs));
        cur.redirects.push(Redirect {
            kind: RedirectKind::HereDoc,
            target: delimiter,
            body: None,
            body_span: None,
            heredoc: Some(id),
        });
    }

    fn heredoc_bodies(&mut self) {
        for (id, delimiter, strip_tabs) in std::mem::take(&mut self.pending) {
            let mut body = String::new();
            let start = self.pos;
            let mut end_of_body = self.chars.len();
            while self.pos < self.chars.len() {
                let end = self.chars[self.pos..]
                    .iter()
                    .position(|&c| c == '\n')
                    .map_or(self.chars.len(), |n| self.pos + n);
                let line: String = self.chars[self.pos..end].iter().collect();
                let line_start = self.pos;
                self.pos = (end + 1).min(self.chars.len());
                let line = if strip_tabs {
                    line.trim_start_matches('\t').to_owned()
                } else {
                    line
                };
                if line == delimiter {
                    end_of_body = line_start;
                    break;
                }
                body.push_str(&line);
                body.push('\n');
            }
            self.bodies[id] = Some((body, start..end_of_body.max(start)));
        }
    }

    fn blanks(&mut self) {
        while matches!(self.peek(), Some(' ' | '\t')) {
            self.pos += 1;
        }
    }

    /// One word, quotes removed; commands substituted inside it go to
    /// `inner`.
    fn word(&mut self, inner: &mut Vec<Command>) -> String {
        let mut out = String::new();
        while let Some(c) = self.peek() {
            match c {
                c if ends_word(c) => break,
                '\\' => {
                    self.pos += 1;
                    match self.peek() {
                        Some('\n') => self.pos += 1,
                        Some(c) => {
                            out.push(c);
                            self.pos += 1;
                        }
                        None => {}
                    }
                }
                '\'' => {
                    self.pos += 1;
                    while let Some(c) = self.peek() {
                        self.pos += 1;
                        if c == '\'' {
                            break;
                        }
                        out.push(c);
                    }
                }
                '"' => {
                    self.pos += 1;
                    self.double_quoted(&mut out, inner);
                }
                '$' | '`' => self.substitution(&mut out, inner),
                _ => {
                    out.push(c);
                    self.pos += 1;
                }
            }
        }
        out
    }

    fn double_quoted(&mut self, out: &mut String, inner: &mut Vec<Command>) {
        while let Some(c) = self.peek() {
            match c {
                '"' => {
                    self.pos += 1;
                    return;
                }
                '\\' if matches!(self.peek_at(1), Some('$' | '`' | '"' | '\\' | '\n')) => {
                    let escaped = self.peek_at(1).unwrap_or_default();
                    if escaped != '\n' {
                        out.push(escaped);
                    }
                    self.pos += 2;
                }
                '$' | '`' => self.substitution(out, inner),
                _ => {
                    out.push(c);
                    self.pos += 1;
                }
            }
        }
    }

    /// At `$` or a backtick: `$(…)` and backticks lexed into `inner`,
    /// `$((…))` and `${…}` kept verbatim, a lone `$` kept.
    fn substitution(&mut self, out: &mut String, inner: &mut Vec<Command>) {
        if self.eat("$((") {
            out.push_str("$((");
            self.verbatim_until("))", out);
        } else if self.eat("${") {
            out.push_str("${");
            self.verbatim_until("}", out);
        } else if self.eat("$(") {
            out.push_str("$(…)");
            inner.extend(substituted(self.list(true)));
        } else if self.eat("`") {
            let mut src = String::new();
            while let Some(c) = self.peek() {
                self.pos += 1;
                match c {
                    '`' => break,
                    '\\' => {
                        if let Some(n) = self.peek() {
                            src.push(n);
                            self.pos += 1;
                        }
                    }
                    _ => src.push(c),
                }
            }
            out.push_str("`…`");
            inner.extend(substituted(parse(&src)).map(|mut c| {
                c.spans.clear();
                c
            }));
        } else {
            out.push('$');
            self.pos += 1;
        }
    }

    fn verbatim_until(&mut self, end: &str, out: &mut String) {
        while self.peek().is_some() {
            if self.eat(end) {
                out.push_str(end);
                return;
            }
            out.push(self.chars[self.pos]);
            self.pos += 1;
        }
    }
}

fn substituted(commands: Vec<Command>) -> impl Iterator<Item = Command> {
    commands.into_iter().map(|mut c| {
        c.substituted = true;
        c
    })
}

/// Shell keywords and wrappers that run the command after them.
const PREFIXES: &[&str] = &[
    "!", "{", "}", "if", "then", "else", "elif", "while", "until", "do", "sudo", "env", "command",
    "time", "nice", "nohup", "exec", "xargs",
];

/// The program a command runs, by base name, and its arguments: past
/// `NAME=value` assignments, shell keywords, wrappers such as `env` or
/// `xargs`, and the wrappers' options.
#[must_use]
pub fn program(words: &[String]) -> Option<(&str, &[String])> {
    let mut i = 0;
    let mut wrapped = false;
    while let Some(w) = words.get(i) {
        let assignment = w
            .split_once('=')
            .is_some_and(|(name, _)| !name.is_empty() && !name.starts_with('-'));
        if assignment || PREFIXES.contains(&w.as_str()) || (wrapped && w.starts_with('-')) {
            wrapped |= !assignment;
            i += 1;
        } else {
            break;
        }
    }
    let name = words.get(i)?;
    Some((name.rsplit('/').next().unwrap_or(name), &words[i + 1..]))
}

/// A `git` invocation: its `-C` directory, subcommand and the arguments
/// after it, the global options skipped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Git<'a> {
    pub dir: Option<&'a str>,
    pub sub: &'a str,
    pub args: &'a [String],
}

#[must_use]
pub fn git(args: &[String]) -> Option<Git<'_>> {
    let mut dir = None;
    let mut i = 0;
    while let Some(w) = args.get(i) {
        match w.as_str() {
            "-C" => {
                dir = args.get(i + 1).map(String::as_str);
                i += 2;
            }
            "-c" | "--git-dir" | "--work-tree" | "--namespace" => i += 2,
            w if w.starts_with('-') => i += 1,
            sub => {
                return Some(Git {
                    dir,
                    sub,
                    args: &args[i + 1..],
                });
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(src: &str) -> Vec<Vec<String>> {
        parse(src).into_iter().map(|c| c.words).collect()
    }

    #[test]
    fn ut_splits_on_operators_outside_quotes() {
        assert_eq!(
            words("a 'x;y' && b \"p|q\" | c; d & e || f"),
            [
                vec!["a", "x;y"],
                vec!["b", "p|q"],
                vec!["c"],
                vec!["d"],
                vec!["e"],
                vec!["f"]
            ]
        );
    }

    #[test]
    fn ut_pipe_marks_the_left_command() {
        let c = parse("cat a.md | head -5");
        assert!(c[0].piped);
        assert!(!c[1].piped);
    }

    #[test]
    fn ut_redirects_and_fd_duplication() {
        let c = &parse("grep x 2>&1 >> out.txt < in 2>/dev/null")[0];
        assert_eq!(c.words, ["grep", "x"]);
        let got: Vec<_> = c
            .redirects
            .iter()
            .map(|r| (r.kind, r.target.as_str()))
            .collect();
        assert_eq!(
            got,
            [
                (RedirectKind::Dup, "1"),
                (RedirectKind::Append, "out.txt"),
                (RedirectKind::Read, "in"),
                (RedirectKind::Write, "/dev/null"),
            ]
        );
    }

    #[test]
    fn ut_heredoc_body_is_not_a_command() {
        let c = parse("git commit -F - <<'EOF'\nfix: x\n\nrm -rf /\nEOF\necho done");
        assert_eq!(c.len(), 2);
        assert_eq!(
            c[0].redirects[0].body.as_deref(),
            Some("fix: x\n\nrm -rf /\n")
        );
        assert_eq!(c[1].words, ["echo", "done"]);
    }

    #[test]
    fn ut_spans_point_into_the_source() {
        let src = "git commit -m \"a b\" <<EOF\nbody\nEOF\n";
        let c = &parse(src)[0];
        let chars: Vec<char> = src.chars().collect();
        let text = |r: &Range<usize>| chars[r.clone()].iter().collect::<String>();
        assert_eq!(text(&c.spans[3]), "\"a b\"");
        assert_eq!(text(c.redirects[0].body_span.as_ref().unwrap()), "body\n");
    }

    #[test]
    fn ut_tab_stripped_heredoc() {
        let c = parse("cat <<-EOF\n\tx\n\tEOF\n");
        assert_eq!(c[0].redirects[0].body.as_deref(), Some("x\n"));
    }

    #[test]
    fn ut_substitutions_are_listed_as_commands() {
        assert_eq!(
            words("echo $(rm -f a) `touch b` \"$(mv c d)\""),
            [
                vec!["echo", "$(…)", "`…`", "$(…)"],
                vec!["rm", "-f", "a"],
                vec!["touch", "b"],
                vec!["mv", "c", "d"]
            ]
        );
    }

    #[test]
    fn ut_arithmetic_and_parameter_expansion_stay_words() {
        assert_eq!(
            words("echo $((1+2)) ${x:-y}"),
            [vec!["echo", "$((1+2))", "${x:-y}"]]
        );
    }

    #[test]
    fn ut_comments_and_continuations() {
        assert_eq!(words("ls \\\n -la # rm x"), [vec!["ls", "-la"]]);
        assert_eq!(words("echo a#b"), [vec!["echo", "a#b"]]);
    }

    #[test]
    fn ut_program_skips_assignments_keywords_and_wrappers() {
        let w: Vec<String> = ["FOO=1", "sudo", "-u", "x", "/bin/rm", "-rf", "t"]
            .map(String::from)
            .to_vec();
        // `-u`'s argument is not known to be one; it is taken as the program.
        assert_eq!(program(&w).map(|p| p.0), Some("x"));
        let w: Vec<String> = ["do", "env", "A=b", "rm", "x"].map(String::from).to_vec();
        assert_eq!(program(&w).map(|p| p.0), Some("rm"));
    }

    #[test]
    fn ut_git_skips_global_options() {
        let w: Vec<String> = ["-C", "/r", "-c", "a=b", "--no-pager", "commit", "-m", "x"]
            .map(String::from)
            .to_vec();
        let g = git(&w).unwrap();
        assert_eq!((g.dir, g.sub, g.args.len()), (Some("/r"), "commit", 2));
    }
}
