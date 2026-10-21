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

use ferrule_core::project::{self, Layout};
use ferrule_core::refusal::Refusal;
use ferrule_core::term::{self, Style};

/// The project's layout, rooted at the main checkout.
pub fn layout() -> Layout {
    Layout::new(project::main_checkout())
}

/// The project's ferrule.toml, from the main checkout.
pub fn config() -> io::Result<ferrule_core::config::Config> {
    ferrule_core::config::Config::load(&project::main_checkout()).map_err(internal)
}

/// Prints a markdown document: as stored, or styled for the terminal.
pub fn doc(out: &mut impl Write, style: Option<Style>, md: &str) -> io::Result<()> {
    match style {
        Some(style) => write!(out, "{}", term::markdown(md, style)),
        None => write!(out, "{md}"),
    }
}

/// Prints a refusal and answers its exit code.
pub fn refuse(out: &mut impl Write, refusal: &Refusal) -> io::Result<u8> {
    writeln!(out, "{refusal}")?;
    Ok(refusal.code())
}

/// An internal failure (the state database, git, the filesystem,
/// ferrule.toml), not a refusal. A command returns it as `Err`; `main`
/// prints `ferrule: <error>` and exits [`exit::INTERNAL`]. A refusal is an
/// answer: the command prints it and returns `Ok` with its exit code.
pub fn internal(e: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> io::Error {
    io::Error::other(e)
}

/// Exit codes every subcommand shares. A refusal's codes come from
/// [`ferrule_core::refusal::Exit`].
pub mod exit {
    use ferrule_core::refusal::Exit;

    /// Done, or a negative answer that is not an error.
    pub const OK: u8 = 0;
    /// Refused or not found.
    pub const REFUSED: u8 = Exit::Refused.code();
    /// Usage, or input the rules reject.
    pub const USAGE: u8 = Exit::Usage.code();
    /// Content the rules refuse: the agent that wrote it fixes it.
    pub const CONTENT: u8 = Exit::Content.code();
    /// An internal failure: the state database, git, the filesystem or
    /// ferrule.toml.
    pub const INTERNAL: u8 = 4;
}
