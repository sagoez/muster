//! Every write a command makes to stdout, and how it ends: a reader that
//! went away, as `| head` does, ends the command quietly; any other failure
//! to write is reported.

use std::{
    io::{self, Write},
    process,
};

use serde::Serialize;

use super::{
    cli_exit::CliExit, error::CliError, output, presenter::Presenter, report::print_error,
};
use crate::constants::APP_NAME;

/// Writes `bytes` to stdout as they are, with no styling stripped or added.
pub fn emit_bytes(bytes: &[u8]) {
    let mut stdout = io::stdout().lock();
    written(stdout.write_all(bytes).and_then(|()| stdout.flush()));
}

/// Writes `text` to stdout, stripping styling where it cannot show.
pub fn emit(text: &str) {
    let mut stdout = anstream::stdout();
    written(
        stdout
            .write_all(text.as_bytes())
            .and_then(|()| stdout.flush()),
    );
}

/// Writes `text` and a line break to stdout, as [`emit`] does.
pub fn emit_line(text: &str) {
    emit(&format!("{text}\n"));
}

/// Ends the command when a write to the person failed.
pub fn written(result: io::Result<()>) {
    if let Err(error) = result {
        end_on_write_error(error);
    }
}

/// Ends the command after its output could not be written: quietly with
/// success when the reader closed the pipe, with a failure otherwise.
fn end_on_write_error(error: io::Error) -> ! {
    if error.kind() == io::ErrorKind::BrokenPipe {
        process::exit(CliExit::Success.code());
    }
    print_error(APP_NAME, &CliError::Output(error));
    process::exit(CliExit::ActionFailed.code());
}

/// Prints `value` as JSON with `json`, through `human` otherwise.
pub fn print_result<T: Serialize>(json: bool, value: &T, human: impl FnOnce(&T)) {
    if !json {
        human(value);
        return;
    }
    match output::json(value) {
        Ok(text) => emit_line(&text),
        Err(error) => fail_command(json, &error),
    }
}

/// Reports `error` as JSON on stdout with `json`, in the CLI's voice on
/// stderr otherwise, and exits with the code its kind documents.
pub fn fail_command(json: bool, error: &CliError) -> ! {
    fail_framed(Presenter::Plain, json, error)
}

/// Reports `error` as JSON on stdout with `json`, otherwise through
/// `presenter`, which closes a frame it opened, and exits with the code its
/// kind documents.
pub fn fail_framed(presenter: Presenter, json: bool, error: &CliError) -> ! {
    if let CliError::Output(source) = error
        && source.kind() == io::ErrorKind::BrokenPipe
    {
        process::exit(CliExit::Success.code());
    }
    let code = CliExit::from(error.kind()).code();
    if json {
        emit_line(&output::failure_json(error));
    } else {
        written(presenter.fail(error));
    }
    process::exit(code);
}
