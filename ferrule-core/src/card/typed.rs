//! The card log's events as code reads them: one variant per event a
//! reader decides from, its words typed, made from the stored structured
//! [`Event`] and never from the rendered body.

use crate::card::board::Event;
use crate::run::tier::Tier;

/// An event code reads, without its words: the stored name a reader
/// filters on in SQL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Name {
    Commit,
    Gauntlet,
    Pushed,
    Pr,
    Ci,
    CiWaived,
    IssueWaived,
    BlockedStart,
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
    Approved,
    Read,
    Overturned,
    TestRed,
    TestPins,
    TestReplaced,
    Posted,
    Rebase,
    LiveCheck,
    Abandoned,
    Tier,
}

impl Name {
    pub const ALL: [Name; 31] = [
        Name::Commit,
        Name::Gauntlet,
        Name::Pushed,
        Name::Pr,
        Name::Ci,
        Name::CiWaived,
        Name::IssueWaived,
        Name::BlockedStart,
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
        Name::Approved,
        Name::Read,
        Name::Overturned,
        Name::TestRed,
        Name::TestPins,
        Name::TestReplaced,
        Name::Posted,
        Name::Rebase,
        Name::LiveCheck,
        Name::Abandoned,
        Name::Tier,
    ];

    /// The name as stored, `=` included for a valued event.
    #[must_use]
    pub const fn stored(self) -> &'static str {
        match self {
            Name::Commit => "commit=",
            Name::Gauntlet => "gauntlet=",
            Name::Pushed => "pushed=",
            Name::Pr => "pr=",
            Name::Ci => "ci=",
            Name::CiWaived => "ci-waived",
            Name::IssueWaived => "issue-waived",
            Name::BlockedStart => "blocked-start",
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
            Name::Approved => "approved=",
            Name::Read => "read",
            Name::Overturned => "overturned",
            Name::TestRed => "test-red",
            Name::TestPins => "test-pins",
            Name::TestReplaced => "test-replaced",
            Name::Posted => "posted=",
            Name::Rebase => "rebase=",
            Name::LiveCheck => "live-check=",
            Name::Abandoned => "abandoned",
            Name::Tier => "tier=",
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
            Name::Rebase => &["base", "from"],
            Name::TestRed => &["test", "mutation"],
            Name::TestPins => &["test"],
            Name::TestReplaced => &["old", "new"],
            Name::Tier => &["promoted"],
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
    /// `ferrule pr push` pushed the commit `sha` after its fast checks.
    Pushed {
        sha: &'a str,
    },
    Pr(PrState),
    Ci(&'a str),
    CiWaived,
    IssueWaived,
    /// The user approved starting a slice with open blockers, which `blockers` lists.
    BlockedStart {
        blockers: &'a str,
    },
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
    Approved(&'a str),
    Read {
        path: Option<&'a str>,
    },
    Overturned {
        count: Option<u64>,
    },
    TestRed {
        test: Option<&'a str>,
        mutation: Option<&'a str>,
    },
    TestPins {
        test: Option<&'a str>,
    },
    TestReplaced {
        old: Option<&'a str>,
        new: Option<&'a str>,
    },
    /// A host write done: `comment` (issue-comment.md, its `sum`, on
    /// `issue`) or `follow-up` (follow-up-`k`.md, filed as `issue`).
    Posted {
        what: &'a str,
        /// The issue as stored: a number or a tracker key.
        issue: Option<&'a str>,
        sum: Option<&'a str>,
        k: Option<u32>,
    },
    /// A rebase of the run's branch, `rebase=<head>`.
    Rebase {
        head: &'a str,
        base: Option<&'a str>,
        from: Option<&'a str>,
    },
    /// The run's live check opened or closed, `live-check=open|closed`.
    LiveCheck {
        open: bool,
    },
    /// The run was abandoned, `abandoned`.
    Abandoned,
    /// A stage's tier decision, `tier=<tier>` with the `promoted` reason.
    Tier {
        tier: Tier,
        promoted: Option<&'a str>,
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
            Typed::Pushed { .. } => Name::Pushed,
            Typed::Pr(_) => Name::Pr,
            Typed::Ci(_) => Name::Ci,
            Typed::CiWaived => Name::CiWaived,
            Typed::IssueWaived => Name::IssueWaived,
            Typed::BlockedStart { .. } => Name::BlockedStart,
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
            Typed::Approved(_) => Name::Approved,
            Typed::Read { .. } => Name::Read,
            Typed::Overturned { .. } => Name::Overturned,
            Typed::TestRed { .. } => Name::TestRed,
            Typed::TestPins { .. } => Name::TestPins,
            Typed::TestReplaced { .. } => Name::TestReplaced,
            Typed::Posted { .. } => Name::Posted,
            Typed::Rebase { .. } => Name::Rebase,
            Typed::LiveCheck { .. } => Name::LiveCheck,
            Typed::Abandoned => Name::Abandoned,
            Typed::Tier { .. } => Name::Tier,
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
            Name::LiveCheck => Typed::LiveCheck {
                open: match value? {
                    "open" => true,
                    "closed" => false,
                    _ => return None,
                },
            },
            Name::Rebase => Typed::Rebase {
                head: value?,
                base: key("base"),
                from: key("from"),
            },
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
            Name::Pushed => Typed::Pushed { sha: value? },
            Name::Pr => Typed::Pr(match value? {
                "draft" => PrState::Draft,
                "ready" => PrState::Ready,
                _ => return None,
            }),
            Name::Ci => Typed::Ci(value?),
            Name::CiWaived => Typed::CiWaived,
            Name::IssueWaived => Typed::IssueWaived,
            Name::Abandoned => Typed::Abandoned,
            Name::Tier => Typed::Tier {
                tier: Tier::parse(value?)?,
                promoted: key("promoted"),
            },
            Name::BlockedStart => Typed::BlockedStart {
                blockers: self.text.as_deref().unwrap_or(""),
            },
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
            Name::Approved => Typed::Approved(value?),
            Name::Read => Typed::Read { path: key("path") },
            Name::Overturned => Typed::Overturned {
                count: key("count").and_then(|c| c.parse().ok()),
            },
            Name::TestRed => Typed::TestRed {
                test: key("test"),
                mutation: key("mutation"),
            },
            Name::TestPins => Typed::TestPins { test: key("test") },
            Name::TestReplaced => Typed::TestReplaced {
                old: key("old"),
                new: key("new"),
            },
            Name::Posted => Typed::Posted {
                what: value?,
                issue: key("issue"),
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
        "files-outside=",
        "test-checks=",
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
        let e = Event::parsed("rebase=abc base=origin/main from=def");
        assert_eq!(
            e.typed(),
            Typed::Rebase {
                head: "abc",
                base: Some("origin/main"),
                from: Some("def"),
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

    #[test]
    fn ut_test_red_reads_its_test_and_mutation() {
        assert_eq!(
            Event::parsed("test-red test=a::t mutation=flip").typed(),
            Typed::TestRed {
                test: Some("a::t"),
                mutation: Some("flip"),
            }
        );
        assert_eq!(
            Event::parsed("test-red MB-R-001 -- t").typed(),
            Typed::TestRed {
                test: None,
                mutation: None,
            }
        );
    }

    #[test]
    fn ut_test_pins_and_replaced_read_their_keys() {
        assert_eq!(
            Event::parsed("test-pins test=a::t -- why").typed(),
            Typed::TestPins { test: Some("a::t") }
        );
        assert_eq!(
            Event::parsed("test-replaced old=a new=b").typed(),
            Typed::TestReplaced {
                old: Some("a"),
                new: Some("b"),
            }
        );
    }

    /// `live-check=open|closed` reads as an open flag, any other value as none.
    #[test]
    fn ut_live_check_event_reads_open_and_closed() {
        assert_eq!(
            Event::parsed("live-check=open").typed(),
            Typed::LiveCheck { open: true }
        );
        assert_eq!(
            Event::parsed("live-check=closed").typed(),
            Typed::LiveCheck { open: false }
        );
        assert_eq!(Event::parsed("live-check=maybe").typed(), Typed::Other);
        assert_eq!(Name::parse("live-check="), Some(Name::LiveCheck));
        assert_eq!(Name::parse("abandoned"), Some(Name::Abandoned));
        assert_eq!(Event::parsed("abandoned").typed(), Typed::Abandoned);
    }
}
