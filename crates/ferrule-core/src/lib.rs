//! Parsers and models shared by the ferrule subcommands. Subcommands never
//! re-parse a workflow file on their own; they go through these modules.

pub mod archive;
pub mod artifacts;
pub mod attribution;
pub mod board;
pub mod cards;
pub mod config;
pub mod git;
pub mod guard;
pub mod markdown;
pub mod project;
pub mod run;
pub mod shell;
pub mod spec;
pub mod spec_diff;
pub mod store;
pub mod vocab;
