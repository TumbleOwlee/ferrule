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
        Command::Card(command) => commands::card::run(command, &mut out),
        Command::Import(args) => commands::card::import(args, &mut out),
        Command::Run(command) => commands::run::run(command, &mut out),
        Command::Prompt(command) => commands::prompt::run(command, &mut out),
        Command::Issue(command) => commands::issue::run(command, &mut out),
        Command::Pr(command) => commands::pr::run(command, &mut out),
        Command::Guide(args) => commands::guide::run(args, &mut out),
        Command::Hook(command) => {
            commands::hook::run(command, &mut std::io::stdin().lock(), &mut out)
        }
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
