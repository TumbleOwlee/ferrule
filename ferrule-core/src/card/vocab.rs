//! The card log's event vocabulary and the validation of one log line
//! against it.

use std::fmt::{self, Write as _};

use crate::refusal::{Exit, Refusal};

/// Which cards an event may appear on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Stage,
    Parent,
    Both,
}

/// The value after `=` of an `<name>=` event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Values {
    /// A bare event, no `=`.
    None,
    Any,
    OneOf(&'static [&'static str]),
}

/// The positional words after the event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Positional {
    None,
    /// A fixed spec ID, then `added`, `changed` or `dropped`.
    IdKind,
    /// An optional comma-separated list of fixed IDs.
    OptionalIds,
}

/// Free text after `--`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Text {
    No,
    Yes,
    Required,
}

/// Who writes an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Owner {
    /// An agent, through `ferrule board log`.
    Agent,
    /// Only a ferrule command, which logs it itself; the command that does.
    Ferrule(&'static str),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Event {
    pub scope: Scope,
    /// The name as written, `=` included for a valued event.
    pub name: &'static str,
    pub values: Values,
    pub positional: Positional,
    pub keys: &'static [&'static str],
    pub text: Text,
    pub owner: Owner,
}

impl Event {
    /// The event, written only by the ferrule command `hint`.
    const fn by(self, hint: &'static str) -> Event {
        Event {
            owner: Owner::Ferrule(hint),
            ..self
        }
    }
}

const fn ev(
    scope: Scope,
    name: &'static str,
    values: Values,
    positional: Positional,
    keys: &'static [&'static str],
    text: Text,
) -> Event {
    Event {
        scope,
        name,
        values,
        positional,
        keys,
        text,
        owner: Owner::Agent,
    }
}

const STATES: &[&str] = &["open", "inprogress", "inreview", "done"];

use Positional as P;
use Scope::{Both, Parent, Stage};
use Values as V;

pub const VOCABULARY: &[Event] = &[
    ev(
        Both,
        "spawn",
        V::None,
        P::None,
        &["agent", "stage"],
        Text::No,
    ),
    ev(Stage, "test-red", V::None, P::OptionalIds, &[], Text::Yes),
    ev(Stage, "green", V::None, P::None, &[], Text::No),
    ev(
        Both,
        "spec-delta",
        V::None,
        P::IdKind,
        &["origin", "quote"],
        Text::Yes,
    ),
    ev(
        Stage,
        "checks=",
        V::OneOf(&["pass", "fail"]),
        P::None,
        &["crates"],
        Text::Yes,
    ),
    ev(Stage, "commit=", V::Any, P::None, &[], Text::No),
    ev(Both, "feedback=", V::Any, P::None, &[], Text::No),
    ev(Stage, "stopped", V::None, P::None, &[], Text::Required),
    ev(Stage, "follow-up", V::None, P::None, &[], Text::Required),
    ev(Stage, "read", V::None, P::None, &["path"], Text::Required),
    ev(Both, "note", V::None, P::None, &[], Text::Required),
    ev(Both, "moved=", V::OneOf(STATES), P::None, &[], Text::No)
        .by("ferrule board move <card> <state>"),
    ev(Stage, "minor=", V::Any, P::None, &[], Text::Yes),
    ev(Stage, "minor-moved=", V::Any, P::None, &[], Text::Yes),
    ev(
        Stage,
        "files+=",
        V::Any,
        P::None,
        &["user", "reason"],
        Text::No,
    ),
    ev(Stage, "auto=", V::Any, P::None, &[], Text::No).by("ferrule stage stop <slug> <n>"),
    ev(Stage, "stop=", V::Any, P::None, &[], Text::No).by("ferrule stage stop <slug> <n>"),
    ev(
        Parent,
        "stop",
        V::None,
        P::None,
        &["kind", "gate", "agent", "count"],
        Text::No,
    )
    .by("ferrule prompt render <slug> <kind>"),
    ev(Parent, "spec-rev=", V::Any, P::None, &[], Text::No)
        .by("ferrule prompt render <slug> approved"),
    ev(Parent, "arch-rev=", V::Any, P::None, &[], Text::No)
        .by("ferrule prompt render <slug> approved"),
    ev(Parent, "decisions-seen=", V::Any, P::None, &[], Text::No)
        .by("ferrule prompt render <slug> approved"),
    ev(Parent, "spec-keep=", V::Any, P::None, &[], Text::No),
    ev(
        Parent,
        "pr=",
        V::OneOf(&["draft", "ready"]),
        P::None,
        &[],
        Text::No,
    )
    .by("ferrule pr push <slug>"),
    ev(
        Parent,
        "gauntlet=",
        V::OneOf(&["pass", "fail"]),
        P::None,
        &["cov", "sha", "step", "reason"],
        Text::No,
    )
    .by("ferrule pr push <slug>"),
    ev(Parent, "replan", V::None, P::None, &["from"], Text::No)
        .by("ferrule stage replan <slug> start"),
    ev(
        Parent,
        "replan=",
        V::OneOf(&["approved", "pre-authorized"]),
        P::None,
        &[],
        Text::No,
    )
    .by("ferrule stage replan <slug> approve"),
    ev(
        Parent,
        "plan-review=",
        V::Any,
        P::None,
        &["count", "round"],
        Text::No,
    )
    .by("ferrule gate next <slug> <gate> <status line>"),
    ev(Parent, "review=", V::Any, P::None, &["scope"], Text::No)
        .by("ferrule gate next <slug> <gate> <status line>"),
    ev(
        Parent,
        "overturned",
        V::None,
        P::None,
        &["scope", "count"],
        Text::Yes,
    ),
    ev(Parent, "rebuild", V::None, P::None, &["reason"], Text::No)
        .by("ferrule run rebuild <issue> <pr>"),
    ev(Parent, "follow-ups=", V::Any, P::None, &[], Text::No).by("ferrule pr push <slug>"),
    ev(Parent, "ci=", V::Any, P::None, &[], Text::No).by("ferrule ci wait <slug>"),
    ev(Parent, "ci-waived", V::None, P::None, &[], Text::Required),
];

/// The longest log line, timestamp included, in chars.
pub const MAX_LINE: usize = 160;
/// `read` events one stage card may carry unless `ferrule.toml` says
/// otherwise.
pub const READ_BUDGET: usize = 3;

#[must_use]
pub fn lookup(name: &str) -> Option<&'static Event> {
    VOCABULARY.iter().find(|e| e.name == name)
}

/// Why a line was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    Event,
    Key,
    Scope,
    Value,
    Length,
    Budget,
    /// An event only a ferrule command writes, refused from `board log`.
    Owner,
    /// A `note` whose text starts with an event's name: that event instead.
    Typed(&'static str),
    /// A stage's `commit=` while the card sits in `inprogress`.
    InProgress,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reject {
    /// The event name without its `=`.
    pub event: String,
    pub reason: Reason,
    /// The event's vocabulary entry, when the name is one.
    pub spec: Option<&'static Event>,
}

/// What `spec` takes, as a one-line hint: its value set, positional words,
/// required keys and `-- text`.
fn usage(spec: &Event) -> String {
    let mut parts: Vec<String> = Vec::new();
    match spec.values {
        Values::OneOf(set) => parts.push(set.join("|")),
        Values::Any => parts.push("a value".to_owned()),
        Values::None => {}
    }
    match spec.positional {
        Positional::IdKind => {
            parts.push("<ID> added|changed|dropped (a fixed ID like MB-R-001)".to_owned());
        }
        Positional::OptionalIds => {
            parts.push("[ids] (fixed IDs like MB-R-001, comma-separated)".to_owned());
        }
        Positional::None => {}
    }
    match spec.name {
        "spec-delta" => parts.push("origin=user|review|impl".to_owned()),
        "stop" => parts.push("kind=question|approval|stage|blocked".to_owned()),
        _ => {}
    }
    let optional: Vec<&str> = spec
        .keys
        .iter()
        .copied()
        .filter(|k| !matches!((spec.name, *k), ("spec-delta", "origin") | ("stop", "kind")))
        .collect();
    if !optional.is_empty() {
        parts.push(format!("optional keys {}", optional.join(",")));
    }
    match spec.text {
        Text::Required => parts.push("-- text (required)".to_owned()),
        Text::Yes => parts.push("optional -- text".to_owned()),
        Text::No => parts.push("no -- text".to_owned()),
    }
    format!("{} takes {}", spec.name, parts.join(", "))
}

impl Reason {
    /// The reason word.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Reason::Event => "event",
            Reason::Key => "key",
            Reason::Scope => "scope",
            Reason::Value => "value",
            Reason::Length => "length",
            Reason::Budget => "budget",
            Reason::Owner => "owner",
            Reason::Typed(_) => "typed",
            Reason::InProgress => "inprogress",
        }
    }
}

impl Reject {
    /// The `card-log=reject` line, exit 2.
    #[must_use]
    pub fn refusal(&self) -> Refusal {
        let r = Refusal::new("card-log", self.reason.as_str())
            .reject()
            .field("event", &self.event)
            .exit(Exit::Usage);
        match self.reason {
            Reason::Typed(use_) => r
                .field("use", use_)
                .hint(format!("log `{use_}` instead of a note")),
            Reason::Owner => {
                let cmd = match self.spec.map(|s| s.owner) {
                    Some(Owner::Ferrule(cmd)) => cmd,
                    _ => "see `ferrule board vocab`",
                };
                r.hint(format!("only ferrule logs it, with `{cmd}`"))
            }
            Reason::InProgress => r
                .field("move", "inreview")
                .hint("move the card to inreview first, then log commit="),
            Reason::Event => r.hint("run `ferrule board vocab` for the events"),
            Reason::Key => r.hint(self.spec.map_or_else(
                || "use only the keys the event lists in `ferrule board vocab`".to_owned(),
                |s| {
                    if s.keys.is_empty() {
                        format!("{} takes no keys", s.name)
                    } else {
                        format!("{} takes only the keys {}", s.name, s.keys.join(","))
                    }
                },
            )),
            Reason::Scope => {
                r.hint("the event is for the other card kind; see `ferrule board vocab`")
            }
            Reason::Value => r.hint(self.spec.map_or_else(
                || {
                    "check the value, positional words and `-- text` against `ferrule board vocab`"
                        .to_owned()
                },
                usage,
            )),
            Reason::Length => r.hint("shorten the line to 160 characters with its timestamp"),
            Reason::Budget => {
                r.hint("the read budget is spent: stop with status=blocked, the plan is too thin")
            }
        }
    }
}

impl fmt::Display for Reject {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.refusal())
    }
}

impl std::error::Error for Reject {}

/// What a line is checked against beyond its own words.
#[derive(Debug, Clone, Copy)]
pub struct Context {
    pub stage: bool,
    pub in_progress: bool,
    /// `read` events already on the card.
    pub reads: usize,
    /// `read` events one stage card may carry.
    pub read_budget: usize,
    /// The line comes from `ferrule board log`, not from a ferrule command.
    pub agent: bool,
}

/// One word after the event: a positional (no key) or a `key=value` pair,
/// the value unquoted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Arg {
    pub key: Option<&'static str>,
    pub value: String,
}

/// An event's words as stored beside its body: the value after the
/// event's `=`, the positionals then the `key=value` pairs, and the free
/// text.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Parts {
    pub value: Option<String>,
    pub args: Vec<Arg>,
    pub text: Option<String>,
}

/// A validated event: its name, its words, and the line body after the
/// timestamp.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub name: &'static str,
    pub parts: Parts,
    pub body: String,
}

fn is_fixed_id(s: &str) -> bool {
    crate::spec::fixed_id_len(s.as_bytes()) == Some(s.len())
}

fn quoted(v: &str) -> String {
    if v.contains(' ') {
        format!("\"{v}\"")
    } else {
        v.to_owned()
    }
}

/// Whether `a` may be positional word `n` of `spec`.
fn positional_ok(spec: &Event, n: usize, a: &str) -> bool {
    match spec.positional {
        P::IdKind if n == 0 => is_fixed_id(a),
        P::IdKind if n == 1 => matches!(a, "added" | "changed" | "dropped"),
        P::OptionalIds if n == 0 => a.split(',').all(is_fixed_id),
        _ => false,
    }
}

/// The body of event `name` with `parts`: the value, positionals, pairs
/// (spaced values quoted) and text, space-separated.
fn render(name: &str, parts: &Parts) -> String {
    let mut body = match &parts.value {
        Some(v) => format!("{name}{}", quoted(v)),
        None => name.to_owned(),
    };
    for a in &parts.args {
        body.push(' ');
        match a.key {
            Some(k) => {
                let _ = write!(body, "{k}={}", quoted(&a.value));
            }
            None => body.push_str(&a.value),
        }
    }
    if let Some(t) = &parts.text {
        body.push(' ');
        body.push_str(t);
    }
    body
}

/// The words after the event, split into positionals, `key=value` pairs
/// and the free text after `--`.
struct Words<'a> {
    positional: Vec<&'a str>,
    kvs: Vec<(&'static str, &'a str)>,
    text: Option<String>,
    /// `spec-delta` carried `origin=`.
    origin: bool,
    /// `stop` carried `kind=`.
    kind: bool,
}

fn words<'a>(spec: &Event, args: &'a [String]) -> Result<Words<'a>, Reason> {
    let mut positional: Vec<&str> = Vec::new();
    let mut kvs: Vec<(&'static str, &str)> = Vec::new();
    let mut text: Option<String> = None;
    let mut origin = false;
    let mut kind = false;
    for a in args {
        if let Some(t) = &mut text {
            if !t.is_empty() {
                t.push(' ');
            }
            t.push_str(a);
            continue;
        }
        if a == "--" {
            text = Some(String::new());
        } else if let Some((k, v)) = a.split_once('=') {
            let Some(k) = spec.keys.iter().copied().find(|key| *key == k) else {
                return Err(Reason::Key);
            };
            if v.is_empty() || v.contains('"') {
                return Err(Reason::Value);
            }
            match (spec.name, k) {
                ("stop", "kind") => {
                    if !matches!(v, "question" | "approval" | "stage" | "blocked") {
                        return Err(Reason::Value);
                    }
                    kind = true;
                }
                ("spec-delta", "origin") => {
                    if !matches!(v, "user" | "review" | "impl") {
                        return Err(Reason::Value);
                    }
                    origin = true;
                }
                _ => {}
            }
            kvs.push((k, v));
        } else {
            if !positional_ok(spec, positional.len(), a) {
                return Err(Reason::Value);
            }
            positional.push(a);
        }
    }
    kvs.sort_by_key(|(k, _)| spec.keys.iter().position(|key| key == k));
    Ok(Words {
        positional,
        kvs,
        text,
        origin,
        kind,
    })
}

/// Validates `event` and `args` (`[pos…] [k=v…] [-- text…]`) and renders
/// the line body.
pub fn check(event: &str, args: &[String], ctx: Context) -> Result<Line, Reject> {
    let (name, value) = match event.split_once('=') {
        Some((n, v)) => (format!("{n}="), Some(v)),
        None => (event.to_owned(), None),
    };
    let spec = lookup(&name);
    let reject = |reason| Reject {
        event: name.trim_end_matches('=').to_owned(),
        reason,
        spec,
    };
    let spec = spec.ok_or_else(|| reject(Reason::Event))?;
    let scope_ok = match spec.scope {
        Both => true,
        Stage => ctx.stage,
        Parent => !ctx.stage,
    };
    if !scope_ok {
        return Err(reject(Reason::Scope));
    }
    if ctx.agent && matches!(spec.owner, Owner::Ferrule(_)) {
        return Err(reject(Reason::Owner));
    }
    if let Some(v) = value {
        let ok = !v.is_empty()
            && !v.contains('"')
            && match spec.values {
                V::OneOf(set) => set.contains(&v),
                _ => true,
            };
        if !ok {
            return Err(reject(Reason::Value));
        }
    }

    let Words {
        positional,
        kvs,
        text,
        origin,
        kind,
    } = words(spec, args).map_err(reject)?;
    if spec.positional == P::IdKind && positional.len() != 2 {
        return Err(reject(Reason::Value));
    }
    if spec.name == "stop" && !kind {
        return Err(reject(Reason::Key));
    }
    if spec.name == "spec-delta" && !origin {
        return Err(reject(Reason::Value));
    }
    if spec.name == "read" && ctx.reads >= ctx.read_budget {
        return Err(reject(Reason::Budget));
    }
    let text = text.filter(|t| !t.is_empty());
    match spec.text {
        Text::No if text.is_some() => return Err(reject(Reason::Value)),
        Text::Required if text.is_none() => return Err(reject(Reason::Value)),
        _ => {}
    }
    if spec.name == "commit=" && ctx.in_progress {
        return Err(reject(Reason::InProgress));
    }
    if spec.name == "note"
        && let Some(t) = &text
        && let Some(use_) = typed_note(t)
    {
        return Err(reject(Reason::Typed(use_)));
    }

    let parts = Parts {
        value: value.map(str::to_owned),
        args: positional
            .into_iter()
            .map(|p| Arg {
                key: None,
                value: p.to_owned(),
            })
            .chain(kvs.into_iter().map(|(k, v)| Arg {
                key: Some(k),
                value: v.to_owned(),
            }))
            .collect(),
        text,
    };
    let body = render(spec.name, &parts);
    if body.contains('\n') {
        return Err(reject(Reason::Value));
    }
    Ok(Line {
        name: spec.name,
        parts,
        body,
    })
}

/// A value at the start of `s`, `"…"`-quoted or up to the next space, and
/// what follows its separating space; `None` for an empty value or an
/// unclosed or run-on quote.
fn take_value(s: &str) -> Option<(&str, &str)> {
    let (v, after) = match s.strip_prefix('"') {
        Some(q) => {
            let (v, after) = q.split_once('"')?;
            if !(after.is_empty() || after.starts_with(' ')) {
                return None;
            }
            (v, after)
        }
        None => s.split_once(' ').map_or((s, ""), |(v, a)| (v, a)),
    };
    let after = after.strip_prefix(' ').unwrap_or(after);
    (!v.is_empty()).then_some((v, after))
}

/// Splits a stored line body back into its event name (through `=` when
/// valued) and its words: positionals and the event's own `key=value`
/// pairs while they last, the rest as text. A body whose event is not in
/// the vocabulary, or whose value does not parse, is all text after its
/// first word.
#[must_use]
pub fn parse(body: &str) -> (&str, Parts) {
    let (word, after) = body.split_once(' ').unwrap_or((body, ""));
    let unparsed = |name| {
        let text = (!after.is_empty()).then(|| after.to_owned());
        (
            name,
            Parts {
                text,
                ..Parts::default()
            },
        )
    };
    let (name, value, mut rest) = match word.find('=') {
        Some(i) => {
            let name = &body[..=i];
            match take_value(&body[i + 1..]) {
                Some((v, rest)) => (name, Some(v.to_owned()), rest),
                None => return unparsed(name),
            }
        }
        None => (word, None, after),
    };
    let Some(spec) = lookup(name) else {
        return unparsed(name);
    };
    let mut args = Vec::new();
    let mut n = 0;
    while !rest.is_empty() {
        let (w, after) = rest.split_once(' ').unwrap_or((rest, ""));
        if let Some((k, _)) = w.split_once('=')
            && let Some(key) = spec.keys.iter().copied().find(|key| *key == k)
        {
            let Some((v, after)) = take_value(&rest[k.len() + 1..]) else {
                break;
            };
            args.push(Arg {
                key: Some(key),
                value: v.to_owned(),
            });
            rest = after;
        } else if positional_ok(spec, n, w) {
            args.push(Arg {
                key: None,
                value: w.to_owned(),
            });
            n += 1;
            rest = after;
        } else {
            break;
        }
    }
    let text = (!rest.is_empty()).then(|| rest.to_owned());
    (name, Parts { value, args, text })
}

/// The event a note's text should have been: its first word, `=…` and a
/// trailing `:` dropped, naming another event, or `…-review` for
/// `review=`.
fn typed_note(text: &str) -> Option<&'static str> {
    let w = text.split(' ').next().unwrap_or("");
    let w = w.split('=').next().unwrap_or("");
    let w = w.strip_suffix(':').unwrap_or(w);
    VOCABULARY
        .iter()
        .find(|e| e.name != "note" && e.name.trim_end_matches('=') == w)
        .map(|e| e.name)
        .or_else(|| w.ends_with("-review").then_some("review="))
}

/// The vocabulary as a table, one event per line.
#[must_use]
pub fn table() -> String {
    table_for(None, None).unwrap_or_default()
}

/// The vocabulary table cut to one event (its name with or without `=`)
/// and to the events one card kind takes (`Both` counts for either); `None`
/// when no event matches.
#[must_use]
pub fn table_for(event: Option<&str>, kind: Option<Scope>) -> Option<String> {
    let mut out = String::from(
        "scope   event            values                         positional  keys                   text      owner\n",
    );
    let wanted = VOCABULARY.iter().filter(|e| {
        event.is_none_or(|n| e.name.trim_end_matches('=') == n.trim_end_matches('='))
            && kind.is_none_or(|k| e.scope == k || e.scope == Both)
    });
    let mut any = false;
    for e in wanted {
        any = true;
        let scope = match e.scope {
            Stage => "stage",
            Parent => "parent",
            Both => "both",
        };
        let values = match e.values {
            V::None => "-".to_owned(),
            V::Any => "any".to_owned(),
            V::OneOf(set) => set.join("|"),
        };
        let positional = match e.positional {
            P::None => "-",
            P::IdKind => "id,kind",
            P::OptionalIds => "[ids]",
        };
        let keys = if e.keys.is_empty() {
            "-".to_owned()
        } else {
            e.keys.join(",")
        };
        let text = match e.text {
            Text::No => "no",
            Text::Yes => "yes",
            Text::Required => "required",
        };
        let owner = match e.owner {
            Owner::Agent => "agent",
            Owner::Ferrule(_) => "ferrule",
        };
        let _ = writeln!(
            out,
            "{scope:<7} {:<16} {values:<30} {positional:<11} {keys:<22} {text:<9} {owner}",
            e.name
        );
    }
    any.then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &str) -> Vec<String> {
        s.split_whitespace().map(str::to_owned).collect()
    }

    const STAGE: Context = Context {
        stage: true,
        in_progress: false,
        reads: 0,
        read_budget: READ_BUDGET,
        agent: false,
    };
    const PARENT: Context = Context {
        stage: false,
        in_progress: false,
        reads: 0,
        read_budget: READ_BUDGET,
        agent: false,
    };

    fn body(event: &str, a: &str, ctx: Context) -> Result<String, String> {
        check(event, &args(a), ctx).map(|l| l.body).map_err(|r| {
            r.to_string()
                .split(" hint=")
                .next()
                .unwrap_or("")
                .to_owned()
        })
    }

    #[test]
    fn ut_reject_ends_in_a_hint() {
        let r = check("nosuch", &[], STAGE).unwrap_err().to_string();
        assert_eq!(
            r,
            "card-log=reject reason=event event=nosuch hint=run `ferrule board vocab` for the events"
        );
    }

    #[test]
    fn ut_value_hint_names_the_events_constraint() {
        let hint = |e, a| {
            let r = check(e, &args(a), STAGE).unwrap_err().to_string();
            r.split(" hint=").nth(1).unwrap().to_owned()
        };
        assert_eq!(
            hint("test-red", "R1"),
            "test-red takes [ids] (fixed IDs like MB-R-001, comma-separated), optional -- text"
        );
        assert_eq!(
            hint("checks=maybe", ""),
            "checks= takes pass|fail, optional keys crates, optional -- text"
        );
        assert_eq!(hint("note", ""), "note takes -- text (required)");
    }

    #[test]
    fn ut_renders_values_positionals_keys_and_text() {
        assert_eq!(
            body(
                "spec-delta",
                "MB-R-002 changed origin=user -- poll retries",
                STAGE
            )
            .unwrap(),
            "spec-delta MB-R-002 changed origin=user poll retries"
        );
        assert_eq!(
            body("gauntlet=pass", "cov=80.1% sha=abc", PARENT).unwrap(),
            "gauntlet=pass cov=80.1% sha=abc"
        );
        assert_eq!(body("green", "", STAGE).unwrap(), "green");
    }

    #[test]
    fn ut_rejects_by_reason() {
        let r = |e, a, c| body(e, a, c).unwrap_err();
        assert_eq!(
            r("nosuch", "", STAGE),
            "card-log=reject reason=event event=nosuch"
        );
        assert_eq!(
            r("green", "", PARENT),
            "card-log=reject reason=scope event=green"
        );
        assert_eq!(
            r("pr=open", "", PARENT),
            "card-log=reject reason=value event=pr"
        );
        assert_eq!(
            r("spawn", "who=x", STAGE),
            "card-log=reject reason=key event=spawn"
        );
        assert_eq!(
            r("note", "", STAGE),
            "card-log=reject reason=value event=note"
        );
        assert_eq!(
            r("green", "-- why", STAGE),
            "card-log=reject reason=value event=green"
        );
        assert_eq!(
            r("stop", "gate=2", PARENT),
            "card-log=reject reason=key event=stop"
        );
        assert_eq!(
            r("spec-delta", "MB-R-002 changed", STAGE),
            "card-log=reject reason=value event=spec-delta"
        );
        assert_eq!(
            r("test-red", "R1", STAGE),
            "card-log=reject reason=value event=test-red"
        );
    }

    #[test]
    fn ut_note_starting_with_an_event_is_refused() {
        assert_eq!(
            body("note", "-- green: all tests pass", STAGE).unwrap_err(),
            "card-log=reject reason=typed event=note use=green"
        );
        assert_eq!(
            body("note", "-- commit=abc landed", STAGE).unwrap_err(),
            "card-log=reject reason=typed event=note use=commit="
        );
        assert_eq!(
            body("note", "-- plan-review: ok", PARENT).unwrap_err(),
            "card-log=reject reason=typed event=note use=plan-review="
        );
        assert!(body("note", "-- renamed the stage", STAGE).is_ok());
    }

    #[test]
    fn ut_budget_and_inprogress_commit() {
        let full = Context { reads: 3, ..STAGE };
        assert_eq!(
            body("read", "path=x -- why", full).unwrap_err(),
            "card-log=reject reason=budget event=read"
        );
        let wip = Context {
            in_progress: true,
            ..STAGE
        };
        assert_eq!(
            body("commit=abc", "", wip).unwrap_err(),
            "card-log=reject reason=inprogress event=commit move=inreview"
        );
    }

    #[test]
    fn ut_spaced_values_are_quoted() {
        assert_eq!(
            body("spec-delta", "MB-R-1 changed quote=a b origin=user", STAGE).unwrap_err(),
            "card-log=reject reason=value event=spec-delta"
        );
        let a = vec![
            "MB-R-1".to_owned(),
            "changed".to_owned(),
            "quote=a b".to_owned(),
            "origin=user".to_owned(),
        ];
        let line = check("spec-delta", &a, STAGE).unwrap();
        assert_eq!(
            line.body,
            "spec-delta MB-R-1 changed origin=user quote=\"a b\""
        );
        assert_eq!(line.parts.args[3], arg(Some("quote"), "a b"));
        assert_eq!(parse(&line.body), ("spec-delta", line.parts));
    }

    fn arg(key: Option<&'static str>, value: &str) -> Arg {
        Arg {
            key,
            value: value.to_owned(),
        }
    }

    #[test]
    fn ut_keys_render_in_vocabulary_order() {
        let line = check("stop", &args("count=2 gate=gate2 kind=approval"), PARENT).unwrap();
        assert_eq!(line.body, "stop kind=approval gate=gate2 count=2");
        assert_eq!(
            line.parts.args,
            [
                arg(Some("kind"), "approval"),
                arg(Some("gate"), "gate2"),
                arg(Some("count"), "2"),
            ]
        );
        let line = check("gauntlet=pass", &args("sha=abc cov=80%"), PARENT).unwrap();
        assert_eq!(line.body, "gauntlet=pass cov=80% sha=abc");
        assert_eq!(line.parts.value.as_deref(), Some("pass"));
    }

    #[test]
    fn ut_text_is_kept_apart_from_the_args() {
        let line = check("note", &args("-- saw gauntlet=pass sha=x"), STAGE).unwrap();
        assert_eq!(line.body, "note saw gauntlet=pass sha=x");
        assert!(line.parts.args.is_empty());
        assert_eq!(line.parts.text.as_deref(), Some("saw gauntlet=pass sha=x"));
        let line = check(
            "spec-delta",
            &args("MB-R-1 added origin=impl -- new rule"),
            STAGE,
        )
        .unwrap();
        assert_eq!(
            line.parts,
            Parts {
                value: None,
                args: vec![
                    arg(None, "MB-R-1"),
                    arg(None, "added"),
                    arg(Some("origin"), "impl"),
                ],
                text: Some("new rule".to_owned()),
            }
        );
    }

    #[test]
    fn ut_parse_splits_stored_bodies() {
        let (name, p) = parse("note saw gauntlet=pass sha=x");
        assert_eq!(name, "note");
        assert!(p.args.is_empty());
        assert_eq!(p.text.as_deref(), Some("saw gauntlet=pass sha=x"));
        let (name, p) = parse("files+=src/a.rs reason=\"a b\" user=dl");
        assert_eq!(name, "files+=");
        assert_eq!(p.value.as_deref(), Some("src/a.rs"));
        assert_eq!(
            p.args,
            [arg(Some("reason"), "a b"), arg(Some("user"), "dl")]
        );
        assert_eq!(p.text, None);
        let (name, p) = parse("minor=\"two words\" why");
        assert_eq!(name, "minor=");
        assert_eq!(p.value.as_deref(), Some("two words"));
        assert_eq!(p.text.as_deref(), Some("why"));
        let (name, p) = parse("bogus=x y z");
        assert_eq!(name, "bogus=");
        assert_eq!(p.value, None);
        assert_eq!(p.text.as_deref(), Some("y z"));
        let (_, p) = parse("files+=a reason=\"open");
        assert_eq!(p.args, []);
        assert_eq!(p.text.as_deref(), Some("reason=\"open"));
        assert_eq!(parse("green"), ("green", Parts::default()));
    }

    #[test]
    fn ut_an_agent_may_not_log_what_only_ferrule_writes() {
        let agent = Context {
            agent: true,
            ..PARENT
        };
        let got = check("gauntlet=pass", &args("sha=x"), agent).unwrap_err();
        assert_eq!(
            got.to_string(),
            "card-log=reject reason=owner event=gauntlet hint=only ferrule logs it, with `ferrule pr push <slug>`"
        );
        assert!(check("gauntlet=pass", &args("sha=x"), PARENT).is_ok());
        assert!(check("ci-waived", &args("-- why"), agent).is_ok());
        assert!(check("spec-keep=MB-R-001", &[], agent).is_ok());
    }

    #[test]
    fn ut_agent_events_are_the_ones_the_guides_have_agents_log() {
        let agent: Vec<&str> = VOCABULARY
            .iter()
            .filter(|e| e.owner == Owner::Agent)
            .map(|e| e.name)
            .collect();
        assert_eq!(
            agent,
            [
                "spawn",
                "test-red",
                "green",
                "spec-delta",
                "checks=",
                "commit=",
                "feedback=",
                "stopped",
                "follow-up",
                "read",
                "note",
                "minor=",
                "minor-moved=",
                "files+=",
                "spec-keep=",
                "overturned",
                "ci-waived",
            ]
        );
    }
}
