use std::io::{self, Read};

use clap::ValueEnum;
use ferrule_core::config::Config;
use ferrule_core::hook::attribution;
use ferrule_core::{git, project};

use crate::commands::exit;

/// Shown after `ferrule hook git --help`.
pub const GIT_AFTER_LONG_HELP: &str = "\
Called by the git hooks `ferrule setup install` writes, not by hand. The main
branch is `main_branch` of ferrule.toml (default main).
Per hook (git passes its arguments and stdin):
  pre-commit           no input; refuses a commit while HEAD is the main
                       branch: `ferrule: commit on '<main>' refused: branch
                       off it, never commit to it.` (stderr, exit 1); create
                       a branch and commit there
  commit-msg <file>    the message file git passes; drops attribution
                       trailers in place and says so on stderr: `ferrule:
                       dropped a tool-attribution trailer from the commit
                       message.` (exit 0). Without <file>: `ferrule hook git
                       commit-msg: the message file is missing` (exit 2)
  pre-push             reads git's `<ref> <sha> <remote ref> <sha>` lines on
                       stdin; refuses a push to the main branch: `ferrule:
                       push to '<main>' refused: merge to it by pull request
                       only.` (stderr, exit 1)
pre-commit and pre-push refuse while ferrule.toml cannot be read:
`ferrule: <error>; refusing until ferrule.toml reads` (stderr, exit 1).
A hook in a checkout without .ferrule/ferrule exits 0 (a shared hooks
directory). Nothing is printed on stdout.
Exit codes: 0 ok, 1 refused (git aborts the operation), 2 usage,
4 internal error.";

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum GitHook {
    /// Refuses a commit on the main branch.
    PreCommit,
    /// Strips attribution trailers from the message file.
    CommitMsg,
    /// Refuses a push of the main branch.
    PrePush,
}

/// Exit 1 refuses the git operation, with the reason on stderr.
pub fn run(hook: GitHook, args: &[String], input: &mut impl Read) -> io::Result<u8> {
    let root = project::main_checkout();
    // A config it cannot read refuses rather than guess the main branch.
    let main = match Config::load(&root) {
        Ok(c) => c.main_branch,
        Err(e) if !matches!(hook, GitHook::CommitMsg) => {
            eprintln!("ferrule: {e}; refusing until ferrule.toml reads");
            return Ok(exit::REFUSED);
        }
        Err(_) => String::new(),
    };
    match hook {
        GitHook::PreCommit => {
            let here = std::env::current_dir()?;
            if git::out(&here, &["symbolic-ref", "--short", "-q", "HEAD"]).as_deref()
                == Some(main.as_str())
            {
                eprintln!(
                    "ferrule: commit on '{main}' refused: branch off it, never commit to it."
                );
                return Ok(exit::REFUSED);
            }
            Ok(exit::OK)
        }
        GitHook::CommitMsg => {
            let Some(file) = args.first() else {
                eprintln!("ferrule hook git commit-msg: the message file is missing");
                return Ok(exit::USAGE);
            };
            let text = std::fs::read_to_string(file)?;
            if let Some(clean) = attribution::strip(&text) {
                std::fs::write(file, clean)?;
                eprintln!("ferrule: dropped a tool-attribution trailer from the commit message.");
            }
            Ok(exit::OK)
        }
        GitHook::PrePush => {
            let mut refs = String::new();
            input.read_to_string(&mut refs)?;
            let target = format!("refs/heads/{main}");
            if refs
                .lines()
                .any(|l| l.split_whitespace().nth(2) == Some(target.as_str()))
            {
                eprintln!("ferrule: push to '{main}' refused: merge to it by pull request only.");
                return Ok(exit::REFUSED);
            }
            Ok(exit::OK)
        }
    }
}
