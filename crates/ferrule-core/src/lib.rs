//! Parsers and models shared by the ferrule subcommands. Subcommands never
//! re-parse a workflow file on their own; they go through these modules.

pub mod approval;
pub mod archive;
pub mod artifacts;
pub mod attribution;
pub mod board;
pub mod body;
pub mod config;
pub mod feedback;
pub mod gate;
pub mod gauntlet;
pub mod git;
pub mod guard;
pub mod markdown;
pub mod next;
pub mod project;
pub mod prompt_lines;
pub mod prompt_render;
pub mod publish;
pub mod rebuild;
pub mod remote;
pub mod request;
pub mod run;
pub mod shell;
pub mod spec;
pub mod spec_diff;
pub mod store;
pub mod vocab;
