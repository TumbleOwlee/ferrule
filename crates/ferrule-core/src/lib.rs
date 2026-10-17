//! Parsers and models shared by the ferrule subcommands. Subcommands never
//! re-parse a workflow file on their own; they go through these modules.

pub mod attribution;
pub mod cards;
pub mod guard;
pub mod markdown;
pub mod project;
pub mod shell;
pub mod spec;
pub mod spec_diff;
