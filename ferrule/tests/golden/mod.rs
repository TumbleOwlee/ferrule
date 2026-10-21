//! Golden tests: a command's stdout plus `exit=<code>` compared with
//! `fixtures/expected/<name>.out`, the same files the workflow's shell tests
//! used. Set `UPDATE=1` to rewrite an expected file from the current output;
//! review every rewritten file in the diff before committing it.

// Each test binary compiles this module and uses only part of it.
#![allow(dead_code)]

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

pub fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

pub struct Run {
    command: Command,
}

impl Run {
    pub fn new<I, S>(args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let mut command = Command::new(env!("CARGO_BIN_EXE_ferrule"));
        command.args(args).env_remove("CLAUDE_PROJECT_DIR");
        Self { command }
    }

    pub fn cwd(mut self, dir: impl AsRef<Path>) -> Self {
        self.command.current_dir(dir);
        self
    }

    pub fn env(mut self, key: &str, value: impl AsRef<OsStr>) -> Self {
        self.command.env(key, value);
        self
    }

    pub fn env_remove(mut self, key: &str) -> Self {
        self.command.env_remove(key);
        self
    }

    pub fn output(mut self) -> Output {
        self.command.output().expect("run ferrule")
    }

    /// Stdout followed by `exit=<code>`, the trailing newline dropped, as the
    /// shell's `$(…; echo "exit=$?")` captured it.
    pub fn text(self) -> String {
        let out = self.output();
        let code = out.status.code().expect("exit code");
        format!("{}exit={code}", String::from_utf8_lossy(&out.stdout))
    }

    pub fn golden(self, name: &str) {
        check(name, &self.text());
    }
}

/// Logs `words` (`<event> [args…]`) on `card` of the project at `root` as a
/// ferrule command would: `board log` refuses the events only ferrule
/// writes, which a test needs on a card as if a command had logged them.
pub fn seed_log(root: &Path, at: &str, card: &str, words: &[&str]) {
    let mut board = ferrule_core::store::Board::open(root, false)
        .unwrap()
        .expect("a board");
    let args: Vec<String> = words[1..].iter().map(|w| (*w).to_owned()).collect();
    board
        .write(|w| w.log(card, words[0], &args, at))
        .unwrap_or_else(|e| panic!("{words:?}: {e:?}"));
}

/// Compares `got` with `expected/<name>.out`, or rewrites it under
/// `UPDATE=1`.
pub fn check(name: &str, got: &str) {
    let path = fixtures().join("expected").join(format!("{name}.out"));
    if std::env::var_os("UPDATE").is_some_and(|v| v == "1") {
        std::fs::write(&path, format!("{got}\n")).unwrap();
        return;
    }
    let want = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(got, want.trim_end_matches('\n'), "golden {name}");
}

/// A fake `gh` on `PATH`: prints the fixture filed under its arguments
/// ([`FakeGh::serve`]), answers any other `gh api` call that sends a body
/// with `{}`, fails the rest, and logs every call.
pub struct FakeGh {
    pub dir: tempfile::TempDir,
}

impl FakeGh {
    pub fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("bin")).unwrap();
        std::fs::create_dir(dir.path().join("fx")).unwrap();
        let script = r#"#!/bin/sh
printf '%s\n' "$*" >> "$GH_DIR/log"
key=$(printf '%s' "$*" | tr -c 'A-Za-z0-9=._-' '_' | cut -c1-120)
if [ -f "$GH_DIR/fx/$key" ]; then
  cat "$GH_DIR/fx/$key"
  [ -f "$GH_DIR/fx/$key.exit" ] && exit "$(cat "$GH_DIR/fx/$key.exit")"
  exit 0
fi
case "$*" in
  "api "*"-f body="*|"api graphql -f id="*) echo '{}'; exit 0 ;;
esac
echo "gh: no fixture for: $*" >&2
exit 1
"#;
        let gh = dir.path().join("bin/gh");
        std::fs::write(&gh, script).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        FakeGh { dir }
    }

    fn key(args: &[&str]) -> String {
        args.join(" ")
            .bytes()
            .map(|b| {
                if b.is_ascii_alphanumeric() || b"=._-".contains(&b) {
                    b as char
                } else {
                    '_'
                }
            })
            .take(120)
            .collect()
    }

    /// `gh <args>` prints `text`.
    pub fn serve(&self, args: &[&str], text: &str) {
        std::fs::write(self.dir.path().join("fx").join(Self::key(args)), text).unwrap();
    }

    /// `gh <args>` prints `text` and exits with `code`.
    pub fn serve_exit(&self, args: &[&str], text: &str, code: i32) {
        self.serve(args, text);
        let exit = self
            .dir
            .path()
            .join("fx")
            .join(format!("{}.exit", Self::key(args)));
        std::fs::write(exit, code.to_string()).unwrap();
    }

    /// `gh <args>` served no more: it falls back to the default answer.
    pub fn clear(&self, args: &[&str]) {
        let fx = self.dir.path().join("fx");
        let _ = std::fs::remove_file(fx.join(Self::key(args)));
        let _ = std::fs::remove_file(fx.join(format!("{}.exit", Self::key(args))));
    }

    /// The calls made, one line each.
    pub fn log(&self) -> String {
        std::fs::read_to_string(self.dir.path().join("log")).unwrap_or_default()
    }

    pub fn on(&self, run: Run) -> Run {
        let path = format!(
            "{}:{}",
            self.dir.path().join("bin").display(),
            std::env::var("PATH").unwrap_or_default()
        );
        run.env("PATH", path).env("GH_DIR", self.dir.path())
    }
}

/// `gh issue view --json …` output for a compact-text fixture (head, url,
/// `by`, blank, body, `--- n comment(s) ---`); comments are dropped.
pub fn gh_json(text: &str) -> String {
    let mut lines = text.lines();
    let head = lines.next().unwrap();
    let url = lines.next().unwrap();
    let by = lines.next().unwrap().strip_prefix("by ").unwrap();
    lines.next();
    let mut body: Vec<&str> = lines.take_while(|l| !l.starts_with("--- ")).collect();
    while body.last().is_some_and(|l| l.is_empty()) {
        body.pop();
    }
    let (num, rest) = head.trim_start_matches('#').split_once(' ').unwrap();
    let (title, state) = rest.rsplit_once(" [").unwrap();
    serde_json::json!({
        "number": num.parse::<u64>().unwrap(),
        "title": title,
        "state": state.trim_end_matches(']'),
        "url": url,
        "author": {"login": by},
        "body": body.join("\n"),
        "comments": [],
    })
    .to_string()
}

pub const GH_FIELDS: &str = "number,title,state,url,author,body,comments";

/// Creates card `id` in `state` with `fields` (`key=value`) as a ferrule
/// command would: `board new` refuses the fields only ferrule sets and a
/// stage starting done, which a test needs on a board.
pub fn seed_card(root: &Path, id: &str, state: &str, fields: &[&str]) {
    let mut board = ferrule_core::store::Board::open(root, true)
        .unwrap()
        .expect("a board");
    let state = ferrule_core::card::board::State::parse(state).expect("a state");
    let fields = key_values(fields);
    board
        .write(|w| w.create(id, state, &fields, "2026-10-01T09:00"))
        .unwrap_or_else(|e| panic!("{id}: {e:?}"));
}

/// Sets `fields` (`key=value`) on card `id` as a ferrule command would.
pub fn seed_set(root: &Path, id: &str, fields: &[&str]) {
    let mut board = ferrule_core::store::Board::open(root, false)
        .unwrap()
        .expect("a board");
    let fields = key_values(fields);
    board
        .write(|w| w.set_all(id, &fields))
        .unwrap_or_else(|e| panic!("{id}: {e:?}"));
}

fn key_values(fields: &[&str]) -> Vec<(String, String)> {
    fields
        .iter()
        .map(|f| {
            let (k, v) = f.split_once('=').expect("key=value");
            (k.to_owned(), v.to_owned())
        })
        .collect()
}
