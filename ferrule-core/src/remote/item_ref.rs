//! A reference to an issue or pull request, parsed once where it comes in
//! (the command line, a card, a host's answer), so a host never builds a
//! URL or API path from raw input.

use std::fmt;
use std::str::FromStr;

use super::Error;

/// An issue or pull request: a number, a Jira key, or a URL ending in a
/// number.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ItemRef {
    /// `42` or `#42`.
    Number(u64),
    /// A Jira key, `KEY-1`: an uppercase letter, uppercase letters or
    /// digits, `-`, digits.
    Key(String),
    /// A GitHub or Bitbucket URL whose last segment is the number: handed
    /// to `gh` as given, only its number ever goes into an API path.
    Url { url: String, number: u64 },
}

/// The refusal of input that is no reference.
fn refused(input: &str) -> Error {
    Error::Ref(format!(
        "'{input}' is not an issue or PR reference: give a number (42), a URL ending in one, \
         or a Jira key (KEY-1)"
    ))
}

/// Whether `s` is a Jira key: `[A-Z][A-Z0-9]+-[0-9]+`.
fn is_key(s: &str) -> bool {
    let Some((project, n)) = s.split_once('-') else {
        return false;
    };
    let mut p = project.chars();
    p.next().is_some_and(|c| c.is_ascii_uppercase())
        && project.len() >= 2
        && p.all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
        && !n.is_empty()
        && n.chars().all(|c| c.is_ascii_digit())
}

fn number(s: &str) -> Option<u64> {
    if s.is_empty() || !s.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

impl ItemRef {
    /// Parses `42`, `#42`, `KEY-1`, or an `http(s)://` URL whose last
    /// path segment is a number (`…/pull/42`) or a Jira key
    /// (`…/browse/KEY-1`, kept as the key); refuses anything else.
    pub fn parse(input: &str) -> Result<Self, Error> {
        let s = input.trim();
        if let Some(n) = number(s.strip_prefix('#').unwrap_or(s)) {
            return Ok(ItemRef::Number(n));
        }
        if is_key(s) {
            return Ok(ItemRef::Key(s.to_owned()));
        }
        let is_url = s.starts_with("https://") || s.starts_with("http://");
        if !is_url
            || s.chars()
                .any(|c| c.is_whitespace() || c.is_control() || c == '"' || c == '\\')
        {
            return Err(refused(input));
        }
        let path = s
            .split(['?', '#'])
            .next()
            .unwrap_or(s)
            .trim_end_matches('/');
        let last = path.rsplit('/').next().unwrap_or("");
        if let Some(n) = number(last) {
            return Ok(ItemRef::Url {
                url: s.to_owned(),
                number: n,
            });
        }
        if is_key(last) {
            return Ok(ItemRef::Key(last.to_owned()));
        }
        Err(refused(input))
    }

    /// The number, for a number or URL.
    #[must_use]
    pub fn number(&self) -> Option<u64> {
        match self {
            ItemRef::Number(n) | ItemRef::Url { number: n, .. } => Some(*n),
            ItemRef::Key(_) => None,
        }
    }

    /// The pull request number; a Jira key names no pull request.
    pub fn pr_number(&self) -> Result<u64, Error> {
        self.number()
            .ok_or_else(|| Error::Ref(format!("{self} is a Jira key, not a pull request")))
    }

    /// The validated segment a host's API path takes: the key or the
    /// number, never the URL.
    #[must_use]
    pub fn segment(&self) -> String {
        match self {
            ItemRef::Key(k) => k.clone(),
            ItemRef::Number(n) | ItemRef::Url { number: n, .. } => n.to_string(),
        }
    }
}

impl From<u64> for ItemRef {
    fn from(n: u64) -> Self {
        ItemRef::Number(n)
    }
}

impl FromStr for ItemRef {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Error> {
        ItemRef::parse(s)
    }
}

impl ItemRef {
    /// How a body cites it: `#12` for a number or URL, the key itself.
    #[must_use]
    pub fn cite(&self) -> String {
        match self {
            ItemRef::Number(n) | ItemRef::Url { number: n, .. } => format!("#{n}"),
            ItemRef::Key(k) => k.clone(),
        }
    }
}

/// A card's `[71, KEY-2]` as references; an item that is none is skipped.
#[must_use]
pub fn refs(list: Option<&str>) -> Vec<ItemRef> {
    slots(list).into_iter().flatten().collect()
}

/// References as a card stores them: `[71, KEY-2]`.
#[must_use]
pub fn ref_list(refs: &[ItemRef]) -> String {
    let items: Vec<String> = refs.iter().map(ToString::to_string).collect();
    format!("[{}]", items.join(", "))
}

/// A card's `[71, -, KEY-3]` by position: item n is slice n's issue, `-`
/// one not filed yet.
#[must_use]
pub fn slots(list: Option<&str>) -> Vec<Option<ItemRef>> {
    let inner = list
        .unwrap_or("")
        .trim()
        .trim_matches(|c| c == '[' || c == ']');
    if inner.trim().is_empty() {
        return Vec::new();
    }
    inner
        .split(',')
        .map(|n| ItemRef::parse(n.trim()).ok())
        .collect()
}

/// Slots as a card stores them: `[-, 72]`.
#[must_use]
pub fn slot_list(slots: &[Option<ItemRef>]) -> String {
    let items: Vec<String> = slots
        .iter()
        .map(|n| {
            n.as_ref()
                .map_or_else(|| "-".to_owned(), ToString::to_string)
        })
        .collect();
    format!("[{}]", items.join(", "))
}

/// The number, key or URL as given (`#` dropped).
impl fmt::Display for ItemRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ItemRef::Number(n) => write!(f, "{n}"),
            ItemRef::Key(k) => f.write_str(k),
            ItemRef::Url { url, .. } => f.write_str(url),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ut_card_lists_hold_numbers_and_keys() {
        assert_eq!(
            refs(Some("[71, #72, KEY-3]")),
            [
                ItemRef::Number(71),
                ItemRef::Number(72),
                ItemRef::Key("KEY-3".into())
            ]
        );
        assert!(refs(None).is_empty());
        assert_eq!(
            ref_list(&[ItemRef::Number(1), ItemRef::Key("A-2".into())]),
            "[1, A-2]"
        );
        assert_eq!(
            slots(Some("[71, -, KEY-3]")),
            [
                Some(ItemRef::Number(71)),
                None,
                Some(ItemRef::Key("KEY-3".into()))
            ]
        );
        assert_eq!(slot_list(&[None, Some(ItemRef::Number(72))]), "[-, 72]");
        assert_eq!(ItemRef::Key("A-3".into()).cite(), "A-3");
        assert_eq!(ItemRef::Number(4).cite(), "#4");
    }

    #[test]
    fn ut_item_ref_parses_numbers_keys_and_urls() {
        assert_eq!(ItemRef::parse("42").unwrap(), ItemRef::Number(42));
        assert_eq!(ItemRef::parse(" #7 ").unwrap(), ItemRef::Number(7));
        assert_eq!(
            ItemRef::parse("PROJ2-15").unwrap(),
            ItemRef::Key("PROJ2-15".into())
        );
        let url = "https://github.com/o/r/pull/34";
        assert_eq!(
            ItemRef::parse(url).unwrap(),
            ItemRef::Url {
                url: url.into(),
                number: 34
            }
        );
        assert_eq!(
            ItemRef::parse("https://bitbucket.org/w/r/pull-requests/9/")
                .unwrap()
                .number(),
            Some(9)
        );
        assert_eq!(
            ItemRef::parse("https://x.atlassian.net/browse/AB-3?focus=1").unwrap(),
            ItemRef::Key("AB-3".into())
        );
        assert_eq!(ItemRef::parse(url).unwrap().to_string(), url);
        assert_eq!(ItemRef::parse("#7").unwrap().to_string(), "7");
    }

    #[test]
    fn ut_item_ref_refuses_what_could_reach_a_path() {
        for bad in [
            "",
            "#",
            "feat/x",
            "A-1",
            "proj-1",
            "PROJ-",
            "PROJ-1/../../admin",
            "1?x=2",
            "../1",
            "https://x/pull/1 2",
            "https://x/pull/abc",
            "99999999999999999999999",
        ] {
            let e = ItemRef::parse(bad).unwrap_err();
            assert!(matches!(e, Error::Ref(_)), "{bad}");
        }
        assert!(
            ItemRef::parse("x")
                .unwrap_err()
                .to_string()
                .contains("Jira key (KEY-1)")
        );
        assert!(ItemRef::Key("KEY-1".into()).pr_number().is_err());
        assert_eq!(ItemRef::Key("KEY-1".into()).segment(), "KEY-1");
        assert_eq!(ItemRef::parse("https://x/pull/5").unwrap().segment(), "5");
    }
}
