//! `ferrule file`: reading files.

use std::io::{self, Write};

use clap::Subcommand;
use ferrule_core::term::Style;

use super::spec::{self, SectionArgs, SectionsArgs};

/// Shown after `ferrule file headings --help`.
const HEADINGS_AFTER_LONG_HELP: &str = "\
Output:
  Each heading line of the file, in file order, fenced code skipped. With
  more than one FILE, each list follows a `== <file> ==` line. --depth <n>
  keeps the headings of level n and above (`--depth 2`: `#` and `##`).
Refusals (stderr, exit 1):
  cannot read <file>: <error>    The file is missing or unreadable; the other
                                 files still list.
Exit codes: 0 ok, 1 a file could not be read, 2 usage, 4 internal error.";

/// Shown after `ferrule file section --help`.
const SECTION_AFTER_LONG_HELP: &str = "\
Output:
  The markdown of each matched section, heading line included, in the order
  the headings were given, trailing blank lines dropped. No status line.
Refusals (stderr, exit 1):
  no section matched:            Followed by the unmatched HEADING texts, one
                                 per line; matched sections still print. List
                                 the file's headings with `ferrule file
                                 headings FILE`, or add --prefix.
  cannot read <file>: <error>    The file is missing or unreadable.
Exit codes: 0 every heading matched, 1 refused, 2 usage, 4 internal error.
Examples:
  ferrule file section Status Scope docs/specs/mb/requirements.md
  ferrule file section Status 'Open questions' docs/specs/x.md
  (headings first, the file last; quote a heading that has spaces)";

#[derive(Debug, Subcommand)]
pub enum FileCommand {
    /// List markdown files' headings, fenced code skipped.
    #[command(after_long_help = HEADINGS_AFTER_LONG_HELP)]
    Headings(SectionsArgs),
    /// Print markdown sections by heading.
    ///
    /// A section runs from its heading to the next heading of equal or
    /// shallower depth. Ask for every heading of one file in one call.
    #[command(after_long_help = SECTION_AFTER_LONG_HELP)]
    Section(SectionArgs),
}

pub fn run(command: &FileCommand, style: Option<Style>, out: &mut impl Write) -> io::Result<u8> {
    match command {
        FileCommand::Headings(args) => spec::sections(args, style, out),
        FileCommand::Section(args) => spec::section(args, style, out),
    }
}
