mod cli;
mod commands;
mod legacy;

use std::io::{self, BufWriter, IsTerminal, Write};
use std::process::ExitCode;

use clap::Parser;

use cli::{Cli, Command, Format};
use ferrule_core::term::Style;

/// The terminal style documents print in; `None` prints markdown.
fn style(format: Format) -> Option<Style> {
    let styled = match format {
        Format::Auto => io::stdout().is_terminal(),
        Format::Text => true,
        Format::Md => false,
    };
    let width = terminal_size::terminal_size()
        .map(|(w, _)| usize::from(w.0))
        .or_else(|| std::env::var("COLUMNS").ok()?.parse().ok())
        .unwrap_or(100);
    styled.then(|| Style {
        color: std::env::var_os("NO_COLOR").is_none_or(|v| v.is_empty()),
        width,
    })
}

fn main() -> ExitCode {
    let args = std::env::args_os().map(|a| a.to_string_lossy().into_owned());
    let cli = Cli::parse_from(legacy::rewrite(args.collect()));
    let style = style(cli.format);
    let mut out = BufWriter::new(io::stdout().lock());
    if let Some(code) = commands::install::pinned(&cli.command) {
        return ExitCode::from(code);
    }
    let result = match &cli.command {
        Command::Setup(command) => commands::install::run(command, &mut out),
        Command::Guide(command) => commands::guide::run(command, style, &mut out),
        Command::Run(command) => commands::run::run(command, &mut out),
        Command::Gate(command) => commands::gate::run(command, &mut out),
        Command::Stage(command) => commands::stage::run(command, &mut out),
        Command::Ci(command) => commands::ci::run(command, &mut out),
        Command::Board(command) => commands::board::run(command, style, &mut out),
        Command::Spec(command) => commands::spec::run(command, style, &mut out),
        Command::File(command) => commands::file::run(command, style, &mut out),
        Command::Prompt(command) => commands::prompt::run(command, &mut out),
        Command::Issue(command) => commands::issue::run(command, style, &mut out),
        Command::Pr(command) => commands::pr::run(command, style, &mut out),
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
