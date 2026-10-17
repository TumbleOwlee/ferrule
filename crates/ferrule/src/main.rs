mod cli;
mod commands;

use std::io::{self, BufWriter, Write};
use std::process::ExitCode;

use clap::Parser;

use cli::{Cli, Command};

fn main() -> ExitCode {
    let cli = Cli::parse();
    let mut out = BufWriter::new(io::stdout().lock());
    let result = match &cli.command {
        Command::Spec(command) => commands::spec::run(command, &mut out),
    }
    .and_then(|code| out.flush().map(|()| code));
    match result {
        Ok(code) => ExitCode::from(code),
        // A reader that stopped early (`| head`) is not a failure.
        Err(e) if e.kind() == io::ErrorKind::BrokenPipe => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("ferrule: {e}");
            ExitCode::from(commands::exit::REFUSED)
        }
    }
}
