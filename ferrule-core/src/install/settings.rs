//! ferrule's entries in `.claude/settings.json`: its hooks and its
//! `permissions.deny` rules, merged into whatever else the file holds.
//! Ferrule's hook entries are found by their command string; its deny rules
//! by the list the lock recorded.

use std::path::Path;

use serde_json::{Map, Value, json};

use super::Error;

pub const PATH: &str = ".claude/settings.json";

const SHIM: &str = "\"$CLAUDE_PROJECT_DIR\"/.ferrule/ferrule hook";

/// (event, entry) for every hook ferrule registers.
fn hooks() -> Vec<(&'static str, Value)> {
    let cmd = |what: &str| json!({"type": "command", "command": format!("{SHIM} {what}")});
    vec![
        (
            "PreToolUse",
            json!({"matcher": "Bash", "hooks": [cmd("shell"), cmd("attribution")]}),
        ),
        ("SessionStart", json!({"hooks": [cmd("session-start")]})),
    ]
}

/// Whether every hook of the entry is ferrule's.
fn ours(entry: &Value) -> bool {
    entry["hooks"].as_array().is_some_and(|hs| {
        !hs.is_empty()
            && hs.iter().all(|h| {
                h["command"]
                    .as_str()
                    .is_some_and(|c| c.contains(".ferrule/ferrule hook"))
            })
    })
}

fn load(root: &Path) -> Result<Map<String, Value>, Error> {
    match std::fs::read_to_string(root.join(PATH)) {
        Ok(t) => match serde_json::from_str(&t) {
            Ok(Value::Object(m)) => Ok(m),
            Ok(_) => Err(Error::Settings("not a JSON object".to_owned())),
            Err(e) => Err(Error::Settings(e.to_string())),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Map::new()),
        Err(e) => Err(e.into()),
    }
}

fn save(root: &Path, m: &Map<String, Value>) -> Result<(), Error> {
    let full = root.join(PATH);
    if let Some(dir) = full.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut text = serde_json::to_string_pretty(m).map_err(|e| Error::Settings(e.to_string()))?;
    text.push('\n');
    std::fs::write(full, text)?;
    Ok(())
}

/// `m[key]` as an object, created when absent.
fn object<'a>(m: &'a mut Map<String, Value>, key: &str) -> &'a mut Map<String, Value> {
    let v = m.entry(key).or_insert_with(|| Value::Object(Map::new()));
    if !v.is_object() {
        *v = Value::Object(Map::new());
    }
    v.as_object_mut().expect("just made an object")
}

fn array<'a>(m: &'a mut Map<String, Value>, key: &str) -> &'a mut Vec<Value> {
    let v = m.entry(key).or_insert_with(|| Value::Array(Vec::new()));
    if !v.is_array() {
        *v = Value::Array(Vec::new());
    }
    v.as_array_mut().expect("just made an array")
}

/// Drops ferrule's hook entries and the `old` deny rules; empty
/// containers ferrule leaves behind go too.
fn strip(m: &mut Map<String, Value>, old: &[String]) {
    if let Some(Value::Object(h)) = m.get_mut("hooks") {
        for (_, entries) in h.iter_mut() {
            if let Value::Array(a) = entries {
                a.retain(|e| !ours(e));
            }
        }
        h.retain(|_, v| v.as_array().is_none_or(|a| !a.is_empty()));
    }
    if m.get("hooks")
        .and_then(Value::as_object)
        .is_some_and(Map::is_empty)
    {
        m.remove("hooks");
    }
    if let Some(Value::Object(p)) = m.get_mut("permissions") {
        if let Some(Value::Array(d)) = p.get_mut("deny") {
            d.retain(|r| r.as_str().is_none_or(|r| !old.iter().any(|o| o == r)));
            if d.is_empty() {
                p.remove("deny");
            }
        }
        if p.is_empty() {
            m.remove("permissions");
        }
    }
}

/// Merges ferrule's entries in; `locked` holds the deny rules ferrule added
/// before and is updated to `deny`. Whether the file changed.
pub fn apply(root: &Path, deny: &[String], locked: &mut Vec<String>) -> Result<bool, Error> {
    let before = load(root)?;
    let mut m = before.clone();
    let mut old = locked.clone();
    old.extend(deny.iter().cloned());
    strip(&mut m, &old);
    for (event, entry) in hooks() {
        array(object(&mut m, "hooks"), event).push(entry);
    }
    let rules = array(object(&mut m, "permissions"), "deny");
    for r in deny {
        rules.push(Value::String(r.clone()));
    }
    *locked = deny.to_vec();
    let changed = m != before;
    if changed {
        save(root, &m)?;
    }
    Ok(changed)
}

/// Every ferrule hook present once, and ferrule's deny rules (`deny`, plus
/// the `locked` ones it added before) exactly `deny`.
pub fn in_step(root: &Path, deny: &[String], locked: &[String]) -> Result<bool, Error> {
    let m = load(root)?;
    let hooks_ok = hooks().iter().all(|(event, entry)| {
        m.get("hooks")
            .and_then(|h| h.get(*event))
            .and_then(Value::as_array)
            .is_some_and(|a| a.iter().filter(|e| ours(e)).eq(std::iter::once(entry)))
    });
    let have: Vec<&str> = m
        .get("permissions")
        .and_then(|p| p.get("deny"))
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let ours: std::collections::BTreeSet<&str> = have
        .into_iter()
        .filter(|r| deny.iter().chain(locked).any(|d| d == r))
        .collect();
    let want: std::collections::BTreeSet<&str> = deny.iter().map(String::as_str).collect();
    Ok(hooks_ok && ours == want)
}

/// Takes ferrule's entries out; whether the file changed.
pub fn remove(root: &Path, locked: &[String]) -> Result<bool, Error> {
    if !root.join(PATH).exists() {
        return Ok(false);
    }
    let before = load(root)?;
    let mut m = before.clone();
    strip(&mut m, locked);
    let changed = m != before;
    if changed {
        save(root, &m)?;
    }
    Ok(changed)
}
