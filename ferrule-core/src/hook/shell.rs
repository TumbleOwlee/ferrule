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
    /// Its stdin is the previous command's output, through `|` or `|&`.
    pub fed: bool,
    /// A construct in it the lexer cannot follow.
    pub opaque: Option<Opaque>,
}

/// What the lexer cannot follow: a command could hide inside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Opaque {
    /// ANSI-C quoting, `$'…'`.
    AnsiC,
    /// An arithmetic command, `((…))`: its `<<` is no heredoc.
    Arithmetic,
    /// `$(…)` or backticks inside `${…}` or `$((…))`, kept verbatim.
    NestedSubstitution,
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
        opaque: None,
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
    /// Seen in the command being lexed; set on it when it ends.
    opaque: Option<Opaque>,
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
        let finish = |cur: &mut Command,
                      out: &mut Vec<Command>,
                      inner: &mut Vec<Command>,
                      opaque: &mut Option<Opaque>| {
            if let Some(o) = opaque.take() {
                cur.opaque.get_or_insert(o);
            }
            if !cur.words.is_empty() || !cur.redirects.is_empty() || cur.opaque.is_some() {
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
                    finish(&mut cur, &mut out, &mut inner, &mut self.opaque);
                    self.heredoc_bodies();
                }
                '&' if matches!(self.peek_at(1), Some('>')) => {
                    self.pos += 1;
                    self.redirect(&mut cur, &mut inner);
                }
                ';' | '&' | '|' => {
                    let piped = self.eat("|&") || (self.peek_at(1) != Some('|') && self.eat("|"));
                    if piped {
                        cur.piped = true;
                    } else if !self.eat("&&") && !self.eat("||") && !self.eat(";;") {
                        self.pos += 1;
                    }
                    finish(&mut cur, &mut out, &mut inner, &mut self.opaque);
                    cur.fed = piped;
                }
                '<' | '>' if self.peek_at(1) == Some('(') => {
                    let start = self.pos;
                    self.pos += 2;
                    inner.extend(substituted(self.list(true)));
                    cur.words.push(format!("{c}(…)"));
                    cur.spans.push(start..self.pos);
                }
                '<' | '>' => self.redirect(&mut cur, &mut inner),
                '(' if self.peek_at(1) == Some('(') => {
                    let start = self.pos;
                    let mut text = String::new();
                    self.pos += 2;
                    text.push_str("((");
                    self.verbatim_until("))", &mut text);
                    self.opaque = Some(Opaque::Arithmetic);
                    cur.words.push(text);
                    cur.spans.push(start..self.pos);
                }
                '(' | ')' => {
                    self.pos += 1;
                    finish(&mut cur, &mut out, &mut inner, &mut self.opaque);
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
        finish(&mut cur, &mut out, &mut inner, &mut self.opaque);
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
        // `< <(…)`: the substitution is the target.
        let target = if matches!(self.peek(), Some('<' | '>')) && self.peek_at(1) == Some('(') {
            let c = self.peek().unwrap_or('<');
            self.pos += 2;
            inner.extend(substituted(self.list(true)));
            format!("{c}(…)")
        } else {
            self.word(inner)
        };
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
                '$' if self.peek_at(1) == Some('\'') => {
                    self.pos += 2;
                    self.opaque = Some(Opaque::AnsiC);
                    while let Some(c) = self.peek() {
                        self.pos += 1;
                        match c {
                            '\'' => break,
                            '\\' => {
                                if let Some(n) = self.peek() {
                                    out.push(n);
                                    self.pos += 1;
                                }
                            }
                            _ => out.push(c),
                        }
                    }
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
            let from = out.len();
            self.verbatim_until("))", out);
            self.nested_substitution(&out[from..]);
        } else if self.eat("${") {
            out.push_str("${");
            let from = out.len();
            self.verbatim_until("}", out);
            self.nested_substitution(&out[from..]);
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

    /// Text kept verbatim that would run a command.
    fn nested_substitution(&mut self, verbatim: &str) {
        if verbatim.contains("$(") || verbatim.contains('`') {
            self.opaque = Some(Opaque::NestedSubstitution);
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

/// Shell keywords that run the command after them.
const KEYWORDS: &[&str] = &[
    "!", "{", "}", "if", "then", "else", "elif", "while", "until", "do",
];

/// A program that runs the command after its options and operands.
struct Wrapper {
    name: &'static str,
    /// Options whose value is the next word.
    valued: &'static [&'static str],
    /// Options after which the wrapper runs no command from its words
    /// (`command -v`), or runs one from a string (`env -S`).
    stops: &'static [&'static str],
    /// Operands before the command: `timeout`'s duration.
    operands: usize,
}

const WRAPPERS: &[Wrapper] = &[
    Wrapper {
        name: "sudo",
        valued: &[
            "-u",
            "-g",
            "-p",
            "-C",
            "-D",
            "-r",
            "-t",
            "-U",
            "-T",
            "-R",
            "--user",
            "--group",
            "--prompt",
            "--chdir",
            "--role",
            "--type",
            "--other-user",
            "--close-from",
        ],
        stops: &[],
        operands: 0,
    },
    Wrapper {
        name: "doas",
        valued: &["-u", "-C"],
        stops: &[],
        operands: 0,
    },
    Wrapper {
        name: "env",
        valued: &["-u", "-C", "--unset", "--chdir"],
        stops: &["-S", "--split-string"],
        operands: 0,
    },
    Wrapper {
        name: "command",
        valued: &[],
        stops: &["-v", "-V"],
        operands: 0,
    },
    Wrapper {
        name: "builtin",
        valued: &[],
        stops: &[],
        operands: 0,
    },
    Wrapper {
        name: "time",
        valued: &["-f", "-o", "--format", "--output"],
        stops: &[],
        operands: 0,
    },
    Wrapper {
        name: "nice",
        valued: &["-n", "--adjustment"],
        stops: &[],
        operands: 0,
    },
    Wrapper {
        name: "nohup",
        valued: &[],
        stops: &[],
        operands: 0,
    },
    Wrapper {
        name: "setsid",
        valued: &[],
        stops: &[],
        operands: 0,
    },
    Wrapper {
        name: "exec",
        valued: &["-a"],
        stops: &[],
        operands: 0,
    },
    Wrapper {
        name: "xargs",
        valued: &[
            "-I",
            "-L",
            "-n",
            "-P",
            "-s",
            "-d",
            "-E",
            "-a",
            "--max-args",
            "--max-procs",
            "--max-chars",
            "--max-lines",
            "--delimiter",
            "--eof",
            "--arg-file",
            "--process-slot-var",
        ],
        stops: &[],
        operands: 0,
    },
    Wrapper {
        name: "timeout",
        valued: &["-s", "-k", "--signal", "--kill-after"],
        stops: &[],
        operands: 1,
    },
    Wrapper {
        name: "stdbuf",
        valued: &["-i", "-o", "-e", "--input", "--output", "--error"],
        stops: &[],
        operands: 0,
    },
    Wrapper {
        name: "ionice",
        valued: &["-c", "-n", "--class", "--classdata"],
        stops: &[],
        operands: 0,
    },
];

impl Wrapper {
    /// How many of `rest` are the wrapper's options and operands; `None`
    /// when an option stops it from wrapping.
    fn skip(&self, rest: &[String]) -> Option<usize> {
        let mut i = 0;
        while let Some(w) = rest.get(i) {
            if self.stops.iter().any(|s| w.starts_with(s)) {
                return None;
            }
            if w == "--" {
                i += 1;
                break;
            }
            if !w.starts_with('-') {
                break;
            }
            i += if self.valued.contains(&w.as_str()) {
                2
            } else {
                1
            };
        }
        Some(i + self.operands)
    }
}

/// The file name of a program path: `/usr/bin/git` → `git`.
#[must_use]
pub fn base(word: &str) -> &str {
    word.rsplit('/').next().unwrap_or(word)
}

/// Where the program sits in `words`, past `NAME=value` assignments, shell
/// keywords and wrappers with their options; and whether `xargs` runs it.
fn locate(words: &[String]) -> (usize, bool) {
    let mut i = 0;
    let mut xargs = false;
    while let Some(w) = words.get(i) {
        let assignment = w
            .split_once('=')
            .is_some_and(|(name, _)| !name.is_empty() && !name.starts_with('-'));
        if assignment || KEYWORDS.contains(&w.as_str()) {
            i += 1;
        } else if w == "coproc" {
            // `coproc NAME { …; }` names the coprocess; `coproc cmd` does not.
            i += if words.get(i + 2).is_some_and(|w| w == "{") {
                2
            } else {
                1
            };
        } else if let Some(n) = WRAPPERS
            .iter()
            .find(|x| x.name == base(w))
            .and_then(|x| x.skip(&words[i + 1..]))
        {
            xargs |= base(w) == "xargs";
            i += 1 + n;
        } else {
            break;
        }
    }
    (i, xargs)
}

/// The program a command runs, by base name, and its arguments: past
/// `NAME=value` assignments, shell keywords, wrappers such as `env`,
/// `timeout` or `xargs`, and the wrappers' options with their values.
#[must_use]
pub fn program(words: &[String]) -> Option<(&str, &[String])> {
    let i = locate(words).0;
    let name = words.get(i)?;
    Some((base(name), &words[i + 1..]))
}

/// Whether the program stands behind anything but shell keywords: an
/// assignment (`A=b cmd`) or a wrapper (`timeout 5 cmd`).
#[must_use]
pub fn wrapped(words: &[String]) -> bool {
    let i = locate(words).0;
    words
        .iter()
        .take(i)
        .any(|w| !KEYWORDS.contains(&w.as_str()))
}

/// Whether `xargs` runs the command's program, adding words from stdin.
#[must_use]
pub fn via_xargs(words: &[String]) -> bool {
    locate(words).1
}

/// Shells: their script is a `-c` string, a file, or stdin.
pub const SHELLS: &[&str] = &["sh", "bash", "dash", "zsh", "ksh", "mksh", "ash", "fish"];

/// Where a shell's script comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Script<'a> {
    /// `-c '…'`.
    Inline(&'a str),
    /// The first operand.
    File(&'a str),
    /// No operand, or `-s`.
    Stdin,
}

/// The script of a shell called with `args`.
#[must_use]
pub fn shell_script(args: &[String]) -> Script<'_> {
    let (mut inline, mut stdin) = (false, false);
    let mut i = 0;
    while let Some(w) = args.get(i) {
        match w.as_str() {
            "--" => {
                i += 1;
                break;
            }
            "-o" | "+o" | "-O" | "+O" | "--rcfile" | "--init-file" => i += 2,
            o if o.starts_with("--") => i += 1,
            o if (o.starts_with('-') || o.starts_with('+')) && o.len() > 1 => {
                inline |= o[1..].contains('c');
                stdin |= o[1..].contains('s');
                i += 1;
            }
            _ => break,
        }
    }
    match args.get(i) {
        Some(s) if inline => Script::Inline(s),
        Some(f) if !stdin => Script::File(f),
        _ => Script::Stdin,
    }
}

/// The shell code a program runs from its words: a shell's `-c` script,
/// `eval`'s words, the `-c` command of `su`, `runuser`, `flock` and
/// `script`, `env -S`'s string with the words after it.
#[must_use]
pub fn scripts(name: &str, args: &[String]) -> Vec<String> {
    match name {
        s if SHELLS.contains(&s) => match shell_script(args) {
            Script::Inline(s) => vec![s.to_owned()],
            _ => Vec::new(),
        },
        "eval" => vec![args.join(" ")],
        "su" | "runuser" | "flock" | "script" => args
            .iter()
            .enumerate()
            .filter_map(|(i, a)| match a.as_str() {
                "-c" | "--command" => args.get(i + 1).cloned(),
                a => a.strip_prefix("--command=").map(str::to_owned),
            })
            .collect(),
        "env" => {
            let Some(i) = args
                .iter()
                .position(|a| a.starts_with("-S") || a.starts_with("--split-string"))
            else {
                return Vec::new();
            };
            let (value, rest) = match args[i].as_str() {
                "-S" | "--split-string" => (args.get(i + 1).cloned().unwrap_or_default(), i + 2),
                a => (
                    a.strip_prefix("--split-string=")
                        .or_else(|| a.strip_prefix("-S"))
                        .unwrap_or_default()
                        .to_owned(),
                    i + 1,
                ),
            };
            let words: Vec<String> = std::iter::once(value)
                .chain(args.iter().skip(rest).cloned())
                .collect();
            vec![words.join(" ")]
        }
        _ => Vec::new(),
    }
}

/// The commands `find -exec`, `-execdir`, `-ok` and `-okdir` run, each up
/// to its `;` or `+`.
#[must_use]
pub fn find_execs(name: &str, args: &[String]) -> Vec<Vec<String>> {
    let mut out = Vec::new();
    if name != "find" {
        return out;
    }
    let mut i = 0;
    while i < args.len() {
        if matches!(args[i].as_str(), "-exec" | "-execdir" | "-ok" | "-okdir") {
            let start = i + 1;
            let end = args[start..]
                .iter()
                .position(|a| a == ";" || a == "+")
                .map_or(args.len(), |n| start + n);
            out.push(args[start..end].to_vec());
            i = end;
        }
        i += 1;
    }
    out
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
            "-c" | "--config-env" | "--git-dir" | "--work-tree" | "--namespace" => i += 2,
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
        assert_eq!(program(&w).map(|p| p.0), Some("rm"));
        let w: Vec<String> = ["do", "env", "A=b", "rm", "x"].map(String::from).to_vec();
        assert_eq!(program(&w).map(|p| p.0), Some("rm"));
    }

    #[test]
    fn ut_program_skips_wrapper_option_values() {
        let p = |src: &str| {
            let c = parse(src);
            program(&c[0].words).map(|(n, a)| (n.to_owned(), a.len()))
        };
        assert_eq!(p("timeout -s KILL 5 git push"), Some(("git".into(), 1)));
        assert_eq!(p("nice -n 5 /usr/bin/git push"), Some(("git".into(), 1)));
        assert_eq!(p("xargs -n 1 -I {} rm {}"), Some(("rm".into(), 1)));
        assert_eq!(p("env -u X A=b exec -a n git"), Some(("git".into(), 0)));
        assert_eq!(p("coproc git push"), Some(("git".into(), 1)));
        assert_eq!(p("command -v git"), Some(("command".into(), 2)));
        assert!(via_xargs(&parse("xargs git")[0].words));
    }

    #[test]
    fn ut_arithmetic_takes_no_heredoc() {
        let c = parse("(( x << 2 ))\ngit push");
        assert_eq!(c[0].opaque, Some(Opaque::Arithmetic));
        assert_eq!(c[1].words, ["git", "push"]);
    }

    #[test]
    fn ut_constructs_the_lexer_cannot_follow() {
        let opaque = |src: &str| parse(src).iter().find_map(|c| c.opaque);
        assert_eq!(opaque("echo $'a\\'b'"), Some(Opaque::AnsiC));
        assert_eq!(opaque("echo \"$'x'\""), None);
        assert_eq!(
            opaque("echo ${x:-$(git push)}"),
            Some(Opaque::NestedSubstitution)
        );
        assert_eq!(
            opaque("echo $(( $(id -u) + 1 ))"),
            Some(Opaque::NestedSubstitution)
        );
        assert_eq!(opaque("echo ${x:-y} $((1+2))"), None);
    }

    #[test]
    fn ut_pipe_feeds_the_right_command_and_process_substitution_is_a_word() {
        let c = parse("echo x | sh; diff <(a) b");
        assert!(c[1].fed);
        assert_eq!(c[2].words, ["diff", "<(…)", "b"]);
        assert_eq!(c[2].spans.len(), 3);
    }

    #[test]
    fn ut_scripts_and_find_execs() {
        let w = |s: &[&str]| s.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        assert_eq!(scripts("bash", &w(&["-lc", "git push", "x"])), ["git push"]);
        assert_eq!(shell_script(&w(&["-s", "a"])), Script::Stdin);
        assert_eq!(shell_script(&w(&["run.sh"])), Script::File("run.sh"));
        assert_eq!(scripts("eval", &w(&["git", "push"])), ["git push"]);
        assert_eq!(scripts("env", &w(&["-S", "git push", "x"])), ["git push x"]);
        assert_eq!(
            find_execs(
                "find",
                &w(&[".", "-exec", "rm", "{}", ";", "-ok", "ls", "+"])
            ),
            [w(&["rm", "{}"]), w(&["ls"])]
        );
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
