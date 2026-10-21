//! The card log's events as code reads them: one variant per event a
//! reader decides from, its words typed, made from the stored structured
//! [`Event`] and never from the rendered body.

use crate::card::board::Event;

/// An event code reads, without its words: the stored name a reader
/// filters on in SQL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Name {
    Commit,
    Gauntlet,
    Pr,
    Ci,
    CiWaived,
    Stop,
    StageStop,
    Auto,
    Replan,
    ReplanApproved,
    SpecDelta,
    SpecRev,
    ArchRev,
    DecisionsSeen,
    SpecKeep,
    Review,
    PlanReview,
    Read,
    Overturned,
    TestRed,
    Posted,
}

impl Name {
    pub const ALL: [Name; 21] = [
        Name::Commit,
        Name::Gauntlet,
        Name::Pr,
        Name::Ci,
        Name::CiWaived,
        Name::Stop,
        Name::StageStop,
        Name::Auto,
        Name::Replan,
        Name::ReplanApproved,
        Name::SpecDelta,
        Name::SpecRev,
        Name::ArchRev,
        Name::DecisionsSeen,
        Name::SpecKeep,
        Name::Review,
        Name::PlanReview,
        Name::Read,
        Name::Overturned,
        Name::TestRed,
        Name::Posted,
    ];

    /// The name as stored, `=` included for a valued event.
    #[must_use]
    pub const fn stored(self) -> &'static str {
        match self {
            Name::Commit => "commit=",
            Name::Gauntlet => "gauntlet=",
            Name::Pr => "pr=",
            Name::Ci => "ci=",
            Name::CiWaived => "ci-waived",
            Name::Stop => "stop",
            Name::StageStop => "stop=",
            Name::Auto => "auto=",
            Name::Replan => "replan",
            Name::ReplanApproved => "replan=",
            Name::SpecDelta => "spec-delta",
            Name::SpecRev => "spec-rev=",
            Name::ArchRev => "arch-rev=",
            Name::DecisionsSeen => "decisions-seen=",
            Name::SpecKeep => "spec-keep=",
            Name::Review => "review=",
            Name::PlanReview => "plan-review=",
            Name::Read => "read",
            Name::Overturned => "overturned",
            Name::TestRed => "test-red",
            Name::Posted => "posted=",
        }
    }

    /// The `key=` words [`Event::typed`] reads off the event.
    #[must_use]
    pub const fn keys(self) -> &'static [&'static str] {
        match self {
            Name::Gauntlet => &["sha", "cov", "step", "reason"],
            Name::Stop => &["kind", "gate"],
            Name::Replan => &["from"],
            Name::SpecDelta => &["origin", "quote"],
            Name::Read => &["path"],
            Name::Overturned => &["count"],
            Name::Posted => &["issue", "sum", "k"],
            _ => &[],
        }
    }

    /// The event stored as `name`, when code reads it.
    #[must_use]
    pub fn parse(name: &str) -> Option<Name> {
        Name::ALL.into_iter().find(|n| n.stored() == name)
    }
}

/// A `gauntlet=` result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Pass,
    Fail,
}

/// A `pr=` state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrState {
    Draft,
    Ready,
}

/// A `replan=` approval.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grant {
    Approved,
    PreAuthorized,
}

/// What a `spec-delta` did to its ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    Added,
    Changed,
    Dropped,
}

impl Change {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Change::Added => "added",
            Change::Changed => "changed",
            Change::Dropped => "dropped",
        }
    }
}

/// A stored log line as code reads it. A line whose name or words do not
/// fit its variant, and every event no code reads, is [`Typed::Other`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Typed<'a> {
    Commit {
        sha: &'a str,
    },
    Gauntlet {
        result: Verdict,
        sha: Option<&'a str>,
        cov: Option<&'a str>,
        step: Option<&'a str>,
        reason: Option<&'a str>,
    },
    Pr(PrState),
    Ci(&'a str),
    CiWaived,
    /// A parent's stop for the user.
    Stop {
        kind: Option<&'a str>,
        gate: Option<&'a str>,
    },
    /// A stage card's stop decision, `stop=<rule>`.
    StageStop(&'a str),
    /// A stage card's auto-continue decision, `auto=…`.
    Auto,
    Replan {
        from: Option<&'a str>,
    },
    ReplanApproved(Grant),
    SpecDelta {
        id: &'a str,
        change: Option<Change>,
        origin: Option<&'a str>,
        quote: Option<&'a str>,
        text: Option<&'a str>,
    },
    SpecRev(&'a str),
    ArchRev(&'a str),
    DecisionsSeen(&'a str),
    SpecKeep,
    Review(&'a str),
    PlanReview(&'a str),
    Read {
        path: Option<&'a str>,
    },
    Overturned {
        count: Option<u64>,
    },
    TestRed,
    /// A host write done: `comment` (issue-comment.md, its `sum`, on
    /// `issue`) or `follow-up` (follow-up-`k`.md, filed as `issue`).
    Posted {
        what: &'a str,
        issue: Option<u64>,
        sum: Option<&'a str>,
        k: Option<u32>,
    },
    Other,
}

impl Typed<'_> {
    /// The event this is, `None` for [`Typed::Other`].
    #[must_use]
    pub fn name(&self) -> Option<Name> {
        Some(match self {
            Typed::Commit { .. } => Name::Commit,
            Typed::Gauntlet { .. } => Name::Gauntlet,
            Typed::Pr(_) => Name::Pr,
            Typed::Ci(_) => Name::Ci,
            Typed::CiWaived => Name::CiWaived,
            Typed::Stop { .. } => Name::Stop,
            Typed::StageStop(_) => Name::StageStop,
            Typed::Auto => Name::Auto,
            Typed::Replan { .. } => Name::Replan,
            Typed::ReplanApproved(_) => Name::ReplanApproved,
            Typed::SpecDelta { .. } => Name::SpecDelta,
            Typed::SpecRev(_) => Name::SpecRev,
            Typed::ArchRev(_) => Name::ArchRev,
            Typed::DecisionsSeen(_) => Name::DecisionsSeen,
            Typed::SpecKeep => Name::SpecKeep,
            Typed::Review(_) => Name::Review,
            Typed::PlanReview(_) => Name::PlanReview,
            Typed::Read { .. } => Name::Read,
            Typed::Overturned { .. } => Name::Overturned,
            Typed::TestRed => Name::TestRed,
            Typed::Posted { .. } => Name::Posted,
            Typed::Other => return None,
        })
    }
}

impl Event {
    /// `key=` of this line as event `name` reads it; a key `name` does not
    /// list is a bug the vocabulary test catches.
    fn key(&self, name: Name, key: &str) -> Option<&str> {
        debug_assert!(name.keys().contains(&key), "{key} not read by {name:?}");
        self.arg(key)
    }

    /// The line as code reads it.
    #[must_use]
    pub fn typed(&self) -> Typed<'_> {
        self.read().unwrap_or(Typed::Other)
    }

    fn read(&self) -> Option<Typed<'_>> {
        let name = Name::parse(&self.name)?;
        let value = self.value.as_deref();
        let key = |k| self.key(name, k);
        Some(match name {
            Name::Commit => Typed::Commit { sha: value? },
            Name::Gauntlet => Typed::Gauntlet {
                result: match value? {
                    "pass" => Verdict::Pass,
                    "fail" => Verdict::Fail,
                    _ => return None,
                },
                sha: key("sha"),
                cov: key("cov"),
                step: key("step"),
                reason: key("reason"),
            },
            Name::Pr => Typed::Pr(match value? {
                "draft" => PrState::Draft,
                "ready" => PrState::Ready,
                _ => return None,
            }),
            Name::Ci => Typed::Ci(value?),
            Name::CiWaived => Typed::CiWaived,
            Name::Stop => Typed::Stop {
                kind: key("kind"),
                gate: key("gate"),
            },
            Name::StageStop => Typed::StageStop(value?),
            Name::Auto => Typed::Auto,
            Name::Replan => Typed::Replan { from: key("from") },
            Name::ReplanApproved => Typed::ReplanApproved(match value? {
                "approved" => Grant::Approved,
                "pre-authorized" => Grant::PreAuthorized,
                _ => return None,
            }),
            Name::SpecDelta => Typed::SpecDelta {
                id: self.positional(0)?,
                change: match self.positional(1) {
                    None => None,
                    Some("added") => Some(Change::Added),
                    Some("changed") => Some(Change::Changed),
                    Some("dropped") => Some(Change::Dropped),
                    Some(_) => return None,
                },
                origin: key("origin"),
                quote: key("quote"),
                text: self.text.as_deref(),
            },
            Name::SpecRev => Typed::SpecRev(value?),
            Name::ArchRev => Typed::ArchRev(value?),
            Name::DecisionsSeen => Typed::DecisionsSeen(value?),
            Name::SpecKeep => Typed::SpecKeep,
            Name::Review => Typed::Review(value?),
            Name::PlanReview => Typed::PlanReview(value?),
            Name::Read => Typed::Read { path: key("path") },
            Name::Overturned => Typed::Overturned {
                count: key("count").and_then(|c| c.parse().ok()),
            },
            Name::TestRed => Typed::TestRed,
            Name::Posted => Typed::Posted {
                what: value?,
                issue: key("issue").and_then(|n| n.parse().ok()),
                sum: key("sum"),
                k: key("k").and_then(|n| n.parse().ok()),
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::card::vocab::{self, Positional, VOCABULARY, Values};

    /// Vocabulary events no code reads: they stay [`Typed::Other`].
    const UNREAD: &[&str] = &[
        "spawn",
        "green",
        "checks=",
        "feedback=",
        "stopped",
        "follow-up",
        "note",
        "moved=",
        "minor=",
        "minor-moved=",
        "files+=",
        "rebuild",
        "follow-ups=",
    ];

    /// A line of `spec` with every key and positional it takes, as
    /// [`Event::typed`] would meet it.
    fn sample(spec: &vocab::Event) -> Event {
        let value = match spec.values {
            Values::None => None,
            Values::Any => Some("1".to_owned()),
            Values::OneOf(set) => Some(set[0].to_owned()),
        };
        let mut args: Vec<(Option<String>, String)> = match spec.positional {
            Positional::None => Vec::new(),
            Positional::IdKind => vec![(None, "MB-R-001".to_owned()), (None, "added".to_owned())],
            Positional::OptionalIds => vec![(None, "MB-R-001".to_owned())],
        };
        args.extend(
            spec.keys
                .iter()
                .map(|k| (Some((*k).to_owned()), "1".to_owned())),
        );
        Event {
            id: 0,
            card: "r".to_owned(),
            at: "2026-10-04T11:50".to_owned(),
            name: spec.name.to_owned(),
            value,
            args,
            text: Some("t".to_owned()),
        }
    }

    #[test]
    fn ut_every_variant_is_a_vocabulary_event_with_its_keys() {
        for n in Name::ALL {
            let spec = vocab::lookup(n.stored())
                .unwrap_or_else(|| panic!("{n:?}: {} not in the vocabulary", n.stored()));
            for k in n.keys() {
                assert!(
                    spec.keys.contains(k),
                    "{n:?} reads {k}, not a key of {}",
                    spec.name
                );
            }
            assert_eq!(Name::parse(n.stored()), Some(n));
        }
    }

    #[test]
    fn ut_every_vocabulary_event_is_a_variant_or_unread() {
        for spec in VOCABULARY {
            let e = sample(spec);
            let typed = e.typed();
            if UNREAD.contains(&spec.name) {
                assert_eq!(typed, Typed::Other, "{} is listed unread", spec.name);
                assert_eq!(Name::parse(spec.name), None);
            } else {
                assert_eq!(
                    typed.name().map(Name::stored),
                    Some(spec.name),
                    "{} is neither a variant nor listed unread",
                    spec.name
                );
            }
        }
        for n in UNREAD {
            assert!(
                vocab::lookup(n).is_some(),
                "{n} listed unread, not in the vocabulary"
            );
        }
    }

    #[test]
    fn ut_typed_reads_the_stored_words() {
        let e = Event::parsed("gauntlet=pass cov=81% sha=abc");
        assert_eq!(
            e.typed(),
            Typed::Gauntlet {
                result: Verdict::Pass,
                sha: Some("abc"),
                cov: Some("81%"),
                step: None,
                reason: None,
            }
        );
        let e = Event::parsed("note gauntlet=pass sha=abc");
        assert_eq!(e.typed(), Typed::Other);
        let e = Event::parsed("spec-delta MB-R-001 changed origin=user why");
        assert_eq!(
            e.typed(),
            Typed::SpecDelta {
                id: "MB-R-001",
                change: Some(Change::Changed),
                origin: Some("user"),
                quote: None,
                text: Some("why"),
            }
        );
    }
}
