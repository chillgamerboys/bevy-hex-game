//! Command-line entry point for deterministic schematic planning and review.

mod cli;
mod render;

use std::ffi::OsString;
use std::io::{self, Write as _};
use std::process::ExitCode;

use cli::{ParseOutcome, USAGE};

fn main() -> ExitCode {
    dispatch(std::env::args_os().skip(1))
}

fn dispatch(arguments: impl IntoIterator<Item = OsString>) -> ExitCode {
    match cli::parse_args(arguments) {
        Ok(ParseOutcome::Help) => write_stdout(USAGE),
        Ok(ParseOutcome::Command(command)) => match cli::execute(command) {
            Ok(summary) => write_stdout(&format!("{summary}\n")),
            Err(error) => write_stderr(&format!("error: {error}\n"), ExitCode::FAILURE),
        },
        Err(error) => write_stderr(&format!("error: {error}\n\n{USAGE}"), ExitCode::from(2)),
    }
}

fn write_stdout(message: &str) -> ExitCode {
    match io::stdout().lock().write_all(message.as_bytes()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => write_stderr(
            &format!("error: write standard output: {error}\n"),
            ExitCode::FAILURE,
        ),
    }
}

fn write_stderr(message: &str, failure: ExitCode) -> ExitCode {
    match io::stderr().lock().write_all(message.as_bytes()) {
        Ok(()) => failure,
        Err(_) => ExitCode::FAILURE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_syntax_uses_the_command_line_error_status() {
        assert_eq!(dispatch([OsString::from("unknown")]), ExitCode::from(2));
    }
}
