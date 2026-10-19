//! The card log's event vocabulary and the validation of one log line
//! against it.

use std::fmt::{self, Write as _};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Event {
    pub scope: Scope,
    /// The name as written, `=` included for a valued event.
    pub name: &'static str,
    pub values: Values,
    pub positional: Positional,
    pub keys: &'static [&'static str],
    pub text: Text,
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
    ev(Both, "moved=", V::OneOf(STATES), P::None, &[], Text::No),
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
    ev(Stage, "auto=", V::Any, P::None, &[], Text::No),
    ev(Stage, "stop=", V::Any, P::None, &[], Text::No),
    ev(
        Parent,
        "stop",
        V::None,
        P::None,
        &["kind", "gate", "agent", "count"],
        Text::No,
    ),
    ev(Parent, "spec-rev=", V::Any, P::None, &[], Text::No),
    ev(Parent, "arch-rev=", V::Any, P::None, &[], Text::No),
    ev(Parent, "decisions-seen=", V::Any, P::None, &[], Text::No),
    ev(Parent, "spec-keep=", V::Any, P::None, &[], Text::No),
    ev(
        Parent,
        "pr=",
        V::OneOf(&["draft", "ready"]),
        P::None,
        &[],
        Text::No,
    ),
    ev(
        Parent,
        "gauntlet=",
        V::OneOf(&["pass", "fail"]),
        P::None,
        &["cov", "sha", "step", "reason"],
        Text::No,
    ),
    ev(Parent, "replan", V::None, P::None, &["from"], Text::No),
    ev(
        Parent,
        "replan=",
        V::OneOf(&["approved", "pre-authorized"]),
        P::None,
        &[],
        Text::No,
    ),
    ev(
        Parent,
        "plan-review=",
        V::Any,
        P::None,
        &["count", "round"],
        Text::No,
    ),
    ev(Parent, "review=", V::Any, P::None, &["scope"], Text::No),
    ev(
        Parent,
        "overturned",
        V::None,
        P::None,
        &["scope", "count"],
        Text::Yes,
    ),
    ev(Parent, "rebuild", V::None, P::None, &["reason"], Text::No),
    ev(Parent, "follow-ups=", V::Any, P::None, &[], Text::No),
    ev(Parent, "ci=", V::Any, P::None, &[], Text::No),
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

impl fmt::Display for Reject {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.reason {
            Reason::Typed(use_) => write!(
                f,
                "card-log=reject event={} reason=typed use={use_}",
                self.event
            ),
            Reason::InProgress => write!(
                f,
                "card-log=reject event={} reason=inprogress move=inreview",
                self.event
            ),
            r => {
                let reason = match r {
                    Reason::Event => "event",
                    Reason::Key => "key",
                    Reason::Scope => "scope",
                    Reason::Value => "value",
                    Reason::Length => "length",
                    _ => "budget",
                };
                let hint = match r {
                    Reason::Event => "run `ferrule card vocab` for the events",
                    Reason::Key => {
                        let hint = self.spec.map_or_else(
                            || {
                                "use only the keys the event lists in `ferrule card vocab`"
                                    .to_owned()
                            },
                            |s| {
                                if s.keys.is_empty() {
                                    format!("{} takes no keys", s.name)
                                } else {
                                    format!("{} takes only the keys {}", s.name, s.keys.join(","))
                                }
                            },
                        );
                        return write!(
                            f,
                            "card-log=reject event={} reason=key hint={hint}",
                            self.event
                        );
                    }
                    Reason::Scope => {
                        "the event is for the other card kind; see `ferrule card vocab`"
                    }
                    Reason::Value => {
                        let hint = self.spec.map_or_else(
                            || {
                                "check the value, positional words and `-- text` against `ferrule card vocab`"
                                    .to_owned()
                            },
                            usage,
                        );
                        return write!(
                            f,
                            "card-log=reject event={} reason=value hint={hint}",
                            self.event
                        );
                    }
                    Reason::Length => "shorten the line to 160 characters with its timestamp",
                    _ => "the read budget is spent: stop with status=blocked, the plan is too thin",
                };
                write!(
                    f,
                    "card-log=reject event={} reason={reason} hint={hint}",
                    self.event
                )
            }
        }
    }
}

/// What a line is checked against beyond its own words.
#[derive(Debug, Clone, Copy)]
pub struct Context {
    pub stage: bool,
    pub in_progress: bool,
    /// `read` events already on the card.
    pub reads: usize,
    /// `read` events one stage card may carry.
    pub read_budget: usize,
}

/// A validated event: its name and the line body after the timestamp.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub name: &'static str,
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

/// The words after the event, split into positionals, `key=value` pairs
/// and the free text after `--`.
struct Words<'a> {
    positional: Vec<&'a str>,
    kvs: Vec<String>,
    text: Option<String>,
    /// `spec-delta` carried `origin=`.
    origin: bool,
    /// `stop` carried `kind=`.
    kind: bool,
}

fn words<'a>(spec: &Event, args: &'a [String]) -> Result<Words<'a>, Reason> {
    let mut positional: Vec<&str> = Vec::new();
    let mut kvs: Vec<String> = Vec::new();
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
            if !spec.keys.contains(&k) {
                return Err(Reason::Key);
            }
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
            kvs.push(format!("{k}={}", quoted(v)));
        } else {
            let n = positional.len();
            let ok = match spec.positional {
                P::IdKind if n == 0 => is_fixed_id(a),
                P::IdKind if n == 1 => matches!(a.as_str(), "added" | "changed" | "dropped"),
                P::OptionalIds if n == 0 => a.split(',').all(is_fixed_id),
                _ => false,
            };
            if !ok {
                return Err(Reason::Value);
            }
            positional.push(a);
        }
    }
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

    let mut body = match value {
        Some(v) => format!("{name}{}", quoted(v)),
        None => name.clone(),
    };
    for p in positional {
        body.push(' ');
        body.push_str(p);
    }
    for kv in kvs {
        body.push(' ');
        body.push_str(&kv);
    }
    if let Some(t) = text {
        body.push(' ');
        body.push_str(&t);
    }
    if body.contains('\n') {
        return Err(reject(Reason::Value));
    }
    Ok(Line {
        name: spec.name,
        body,
    })
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
    let mut out = String::from(
        "scope   event            values                         positional  keys                   text\n",
    );
    for e in VOCABULARY {
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
        let _ = writeln!(
            out,
            "{scope:<7} {:<16} {values:<30} {positional:<11} {keys:<22} {text}",
            e.name
        );
    }
    out
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
    };
    const PARENT: Context = Context {
        stage: false,
        in_progress: false,
        reads: 0,
        read_budget: READ_BUDGET,
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
            "card-log=reject event=nosuch reason=event hint=run `ferrule card vocab` for the events"
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
            "card-log=reject event=nosuch reason=event"
        );
        assert_eq!(
            r("green", "", PARENT),
            "card-log=reject event=green reason=scope"
        );
        assert_eq!(
            r("pr=open", "", PARENT),
            "card-log=reject event=pr reason=value"
        );
        assert_eq!(
            r("spawn", "who=x", STAGE),
            "card-log=reject event=spawn reason=key"
        );
        assert_eq!(
            r("note", "", STAGE),
            "card-log=reject event=note reason=value"
        );
        assert_eq!(
            r("green", "-- why", STAGE),
            "card-log=reject event=green reason=value"
        );
        assert_eq!(
            r("stop", "gate=2", PARENT),
            "card-log=reject event=stop reason=key"
        );
        assert_eq!(
            r("spec-delta", "MB-R-002 changed", STAGE),
            "card-log=reject event=spec-delta reason=value"
        );
        assert_eq!(
            r("test-red", "R1", STAGE),
            "card-log=reject event=test-red reason=value"
        );
    }

    #[test]
    fn ut_note_starting_with_an_event_is_refused() {
        assert_eq!(
            body("note", "-- green: all tests pass", STAGE).unwrap_err(),
            "card-log=reject event=note reason=typed use=green"
        );
        assert_eq!(
            body("note", "-- commit=abc landed", STAGE).unwrap_err(),
            "card-log=reject event=note reason=typed use=commit="
        );
        assert_eq!(
            body("note", "-- plan-review: ok", PARENT).unwrap_err(),
            "card-log=reject event=note reason=typed use=plan-review="
        );
        assert!(body("note", "-- renamed the stage", STAGE).is_ok());
    }

    #[test]
    fn ut_budget_and_inprogress_commit() {
        let full = Context { reads: 3, ..STAGE };
        assert_eq!(
            body("read", "path=x -- why", full).unwrap_err(),
            "card-log=reject event=read reason=budget"
        );
        let wip = Context {
            in_progress: true,
            ..STAGE
        };
        assert_eq!(
            body("commit=abc", "", wip).unwrap_err(),
            "card-log=reject event=commit reason=inprogress move=inreview"
        );
    }

    #[test]
    fn ut_spaced_values_are_quoted() {
        assert_eq!(
            body("spec-delta", "MB-R-1 changed quote=a b origin=user", STAGE).unwrap_err(),
            "card-log=reject event=spec-delta reason=value"
        );
        let a = vec![
            "MB-R-1".to_owned(),
            "changed".to_owned(),
            "quote=a b".to_owned(),
            "origin=user".to_owned(),
        ];
        assert_eq!(
            check("spec-delta", &a, STAGE).unwrap().body,
            "spec-delta MB-R-1 changed quote=\"a b\" origin=user"
        );
    }
}
