use clap::{CommandFactory, FromArgMatches};
use simplenote_cli::{
    Error,
    api::Http,
    auth,
    cli::{Cli, Command, Output, execute, render},
};
use std::{
    io::{self, IsTerminal, Write},
    process::ExitCode,
};

fn main() -> ExitCode {
    let name = env!("CARGO_BIN_NAME");
    let cli = match Cli::command()
        .name(name)
        .bin_name(name)
        .try_get_matches()
        .and_then(|matches| Cli::from_arg_matches(&matches))
    {
        Ok(cli) => cli,
        Err(error)
            if matches!(
                error.kind(),
                clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
            ) =>
        {
            return match error.print() {
                Ok(()) => ExitCode::SUCCESS,
                Err(_) => ExitCode::from(8),
            };
        }
        Err(error) => {
            let args: Vec<String> = std::env::args().collect();
            let mut output = Output::Auto;
            for (i, arg) in args
                .iter()
                .enumerate()
                .take_while(|(_, arg)| arg.as_str() != "--")
            {
                let value = arg
                    .strip_prefix("--output=")
                    .or_else(|| arg.strip_prefix("-o="))
                    .or_else(|| arg.strip_prefix("-o").filter(|s| !s.is_empty()))
                    .or_else(|| {
                        if arg == "--output" || arg == "-o" {
                            args.get(i + 1).map(String::as_str)
                        } else {
                            None
                        }
                    });
                if let Some(value) = value {
                    output = match value {
                        "text" => Output::Text,
                        "json" => Output::Json,
                        _ => Output::Auto,
                    };
                }
            }
            return fail(Error::usage(error.to_string()), output.json());
        }
    };
    let result = (|| {
        // Introspection and completion never require a home directory or HTTP client.
        match cli.command {
            Command::Schema => {
                return render(
                    &simplenote_cli::schema::contract(),
                    true,
                    &mut io::stdout().lock(),
                );
            }
            Command::Completions { shell } => {
                clap_complete::generate(shell, &mut Cli::command(), name, &mut io::stdout());
                return Ok(());
            }
            Command::Tui {
                demo: true,
                snapshot,
                width,
                height,
            } => {
                if snapshot {
                    io::stdout()
                        .write_all(simplenote_cli::tui::snapshot(width, height)?.as_bytes())?;
                } else {
                    simplenote_cli::tui::run(None)?;
                }
                return Ok(());
            }
            _ => {}
        }
        let http = Http::new()?;
        let dir = auth::config_dir()?;
        if let Some(value) = execute(&cli, &http, &dir)? {
            render(&value, cli.output.json(), &mut io::stdout().lock())?;
        }
        Ok(())
    })();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => fail(e, cli.output.json()),
    }
}
fn fail(error: Error, json: bool) -> ExitCode {
    if json {
        let _ = writeln!(io::stderr(), "{}", error.envelope());
    } else {
        let _ = writeln!(io::stderr(), "{}: {}", error.kind, error.message);
    }
    if io::stdout().is_terminal() {
        let _ = io::stdout().flush();
    }
    ExitCode::from(error.exit_code())
}
