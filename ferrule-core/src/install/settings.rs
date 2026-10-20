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

/// `m[key]` as an object, created when absent; refused when it is something
/// else, so that nothing of the user's is overwritten.
fn object<'a>(
    m: &'a mut Map<String, Value>,
    key: &str,
) -> Result<&'a mut Map<String, Value>, Error> {
    match m.entry(key).or_insert_with(|| Value::Object(Map::new())) {
        Value::Object(o) => Ok(o),
        _ => Err(Error::Settings(format!("{key} is not an object"))),
    }
}

/// `m[key]` as an array, created when absent; refused when it is something
/// else.
fn array<'a>(m: &'a mut Map<String, Value>, key: &str) -> Result<&'a mut Vec<Value>, Error> {
    match m.entry(key).or_insert_with(|| Value::Array(Vec::new())) {
        Value::Array(a) => Ok(a),
        _ => Err(Error::Settings(format!("{key} is not an array"))),
    }
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
/// before and becomes the ones it adds now: a rule the file already holds
/// (the user's own) is left alone and not recorded. Whether the file changed.
pub fn apply(root: &Path, deny: &[String], locked: &mut Vec<String>) -> Result<bool, Error> {
    let before = load(root)?;
    let mut m = before.clone();
    strip(&mut m, locked);
    for (event, entry) in hooks() {
        array(object(&mut m, "hooks")?, event)?.push(entry);
    }
    let rules = array(object(&mut m, "permissions")?, "deny")?;
    let mut added = Vec::new();
    for r in deny {
        if !rules.iter().any(|v| v.as_str() == Some(r)) {
            rules.push(Value::String(r.clone()));
            added.push(r.clone());
        }
    }
    *locked = added;
    let changed = m != before;
    if changed {
        save(root, &m)?;
    }
    Ok(changed)
}

/// Every ferrule hook present once, every rule of `deny` present, and none
/// of the `locked` rules ferrule added before that `deny` has dropped.
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
    let all = deny.iter().all(|d| have.contains(&d.as_str()));
    let stale = locked
        .iter()
        .any(|l| !deny.contains(l) && have.contains(&l.as_str()));
    Ok(hooks_ok && all && !stale)
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
