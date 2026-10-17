//! Parsers and models shared by the ferrule subcommands. Subcommands never
//! re-parse a workflow file on their own; they go through these modules.

pub mod cards;
pub mod markdown;
pub mod project;
pub mod spec;
pub mod spec_diff;
