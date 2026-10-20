//! The one shape of a refusal: `<verb>=refused reason=<r>[ k=v…][ detail="<d>"] hint=<fix>`.
//!
//! A refusal is an answer, not a failure: the command did what it was asked
//! and the rules said no. The hint names the fix and runs to the end of the
//! line, so it always comes last.

use std::borrow::Cow;
use std::fmt;

/// The exit code a refusal ends the command with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Exit {
    /// Refused or not found.
    Refused = 1,
    /// Usage, or input the rules reject.
    Usage = 2,
    /// Content the rules refuse: the agent that wrote it fixes it.
    Content = 3,
}

impl Exit {
    #[must_use]
    pub const fn code(self) -> u8 {
        self as u8
    }
}

/// The word after `<verb>=`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// `<verb>=refused`: the command refused.
    Refused,
    /// `<verb>=reject`: the input was rejected (`card-log=reject`).
    Reject,
}

/// A refusal, rendered by its `Display`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    verb: Cow<'static, str>,
    verdict: Verdict,
    reason: Cow<'static, str>,
    fields: Vec<(&'static str, String)>,
    detail: Option<String>,
    hint: Option<String>,
    exit: Exit,
}

impl Refusal {
    /// `<verb>=refused reason=<reason>`, exit 1, with no fields, detail or hint yet.
    #[must_use]
    pub fn new(verb: impl Into<Cow<'static, str>>, reason: impl Into<Cow<'static, str>>) -> Self {
        Refusal {
            verb: verb.into(),
            verdict: Verdict::Refused,
            reason: reason.into(),
            fields: Vec::new(),
            detail: None,
            hint: None,
            exit: Exit::Refused,
        }
    }

    /// Prints `<verb>=reject` instead of `<verb>=refused`.
    #[must_use]
    pub fn reject(mut self) -> Self {
        self.verdict = Verdict::Reject;
        self
    }

    /// A ` <key>=<value>` after the reason, in the order added.
    #[must_use]
    pub fn field(mut self, key: &'static str, value: impl fmt::Display) -> Self {
        self.fields.push((key, value.to_string()));
        self
    }

    /// What git or the host said, as ` detail="<text>"` on one line:
    /// whitespace runs collapse to one space and `"` becomes `'`. Blank
    /// text adds nothing.
    #[must_use]
    pub fn detail(mut self, text: impl AsRef<str>) -> Self {
        let one = text
            .as_ref()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .replace('"', "'");
        self.detail = (!one.is_empty()).then_some(one);
        self
    }

    /// The fix, printed last as ` hint=<fix>`; newlines become spaces.
    #[must_use]
    pub fn hint(mut self, fix: impl Into<String>) -> Self {
        let fix: String = fix.into();
        self.hint = Some(fix.split_whitespace().collect::<Vec<_>>().join(" "));
        self
    }

    /// The exit code; [`Exit::Refused`] unless set.
    #[must_use]
    pub fn exit(mut self, exit: Exit) -> Self {
        self.exit = exit;
        self
    }

    #[must_use]
    pub fn verb(&self) -> &str {
        &self.verb
    }

    #[must_use]
    pub fn reason(&self) -> &str {
        &self.reason
    }

    #[must_use]
    pub fn code(&self) -> u8 {
        self.exit.code()
    }
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let verdict = match self.verdict {
            Verdict::Refused => "refused",
            Verdict::Reject => "reject",
        };
        write!(f, "{}={verdict} reason={}", self.verb, self.reason)?;
        for (k, v) in &self.fields {
            write!(f, " {k}={v}")?;
        }
        if let Some(d) = &self.detail {
            write!(f, " detail=\"{d}\"")?;
        }
        if let Some(h) = &self.hint {
            write!(f, " hint={h}")?;
        }
        Ok(())
    }
}

impl std::error::Error for Refusal {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reason_then_hint() {
        let r = Refusal::new("diff", "card").hint("check the slug with `ferrule board list`");
        assert_eq!(
            r.to_string(),
            "diff=refused reason=card hint=check the slug with `ferrule board list`"
        );
        assert_eq!(r.code(), 1);
    }

    #[test]
    fn fields_in_order_then_detail_then_hint() {
        let r = Refusal::new("push", "rejected")
            .field("branch", "feat/x")
            .field("n", 3)
            .detail("! [rejected]  main -> main\n  (fetch first)\n")
            .hint("pull, then push again");
        assert_eq!(
            r.to_string(),
            "push=refused reason=rejected branch=feat/x n=3 \
             detail=\"! [rejected] main -> main (fetch first)\" hint=pull, then push again"
        );
    }

    #[test]
    fn detail_quotes_and_blank() {
        let r = Refusal::new("x", "y").detail("say \"no\"").hint("h");
        assert_eq!(
            r.to_string(),
            "x=refused reason=y detail=\"say 'no'\" hint=h"
        );
        let r = Refusal::new("x", "y").detail(" \n\t ").hint("h");
        assert_eq!(r.to_string(), "x=refused reason=y hint=h");
    }

    #[test]
    fn hint_is_one_line_and_last() {
        let r = Refusal::new("x", "y").hint("a\nb").field("k", "v");
        assert_eq!(r.to_string(), "x=refused reason=y k=v hint=a b");
    }

    #[test]
    fn reject_and_exit() {
        let r = Refusal::new("card-log", "owner")
            .reject()
            .field("event", "moved")
            .exit(Exit::Usage);
        assert_eq!(r.to_string(), "card-log=reject reason=owner event=moved");
        assert_eq!(r.code(), 2);
        assert_eq!(r.verb(), "card-log");
        assert_eq!(r.reason(), "owner");
    }
}
