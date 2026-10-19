use std::io::{self, Read};

use clap::ValueEnum;
use ferrule_core::config::Config;
use ferrule_core::hook::attribution;
use ferrule_core::{git, project};

use crate::commands::exit;

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum GitHook {
    PreCommit,
    CommitMsg,
    PrePush,
}

/// Exit 1 refuses the git operation, with the reason on stderr.
pub fn run(hook: GitHook, args: &[String], input: &mut impl Read) -> io::Result<u8> {
    let root = project::main_checkout();
    let main = Config::load(&root).map_or_else(|_| "main".to_owned(), |c| c.main_branch);
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
