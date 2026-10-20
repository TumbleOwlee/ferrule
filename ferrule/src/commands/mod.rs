pub mod board;
pub mod ci;
pub mod file;
pub mod gate;
pub mod guide;
pub mod hook;
pub mod install;
pub mod issue;
pub mod pr;
pub mod prompt;
pub mod run;
pub mod spec;
pub mod stage;
pub mod uninstall;

use std::io::{self, Write};

use ferrule_core::term::{self, Style};

/// Prints a markdown document: as stored, or styled for the terminal.
pub fn doc(out: &mut impl Write, style: Option<Style>, md: &str) -> io::Result<()> {
    match style {
        Some(style) => write!(out, "{}", term::markdown(md, style)),
        None => write!(out, "{md}"),
    }
}

/// Exit codes every subcommand shares.
pub mod exit {
    /// Done, or a negative answer that is not an error.
    pub const OK: u8 = 0;
    /// Refused or not found.
    pub const REFUSED: u8 = 1;
    /// Usage, or input the rules reject.
    pub const USAGE: u8 = 2;
    /// Content the rules refuse: the agent that wrote it fixes it.
    pub const CONTENT: u8 = 3;
}
