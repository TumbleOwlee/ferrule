//! Parsers and models shared by the ferrule subcommands. Subcommands never
//! re-parse a workflow file on their own; they go through these modules.
//! A subcommand's own models live under its module (`card`, `hook`,
//! `install`, `issue`, `pr`, `prompt`, `run`, `spec`); the rest are shared.

pub mod artifacts;
pub mod body;
pub mod card;
pub mod config;
pub mod git;
pub mod guide;
pub mod hook;
pub mod install;
pub mod issue;
pub mod markdown;
pub mod pr;
pub mod project;
pub mod prompt;
pub mod remote;
pub mod run;
pub mod spec;
pub mod store;
pub mod term;
