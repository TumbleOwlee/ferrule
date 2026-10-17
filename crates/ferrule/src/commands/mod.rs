pub mod card;
pub mod hook;
pub mod spec;

/// Exit codes every subcommand shares.
pub mod exit {
    /// Done, or a negative answer that is not an error.
    pub const OK: u8 = 0;
    /// Refused or not found.
    pub const REFUSED: u8 = 1;
    /// Usage, or input the rules reject.
    pub const USAGE: u8 = 2;
}
