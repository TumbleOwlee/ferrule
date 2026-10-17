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
