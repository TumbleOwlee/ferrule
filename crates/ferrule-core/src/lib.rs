//! Parsers and models shared by the ferrule subcommands. Subcommands never
//! re-parse a workflow file on their own; they go through these modules.

pub mod markdown;
pub mod project;
pub mod spec;
