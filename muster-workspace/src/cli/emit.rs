//! Every write a command makes to stdout, and how it ends: a reader that
//! went away, as `| head` does, ends the command quietly with the code its
//! outcome already set; any other failure to write is reported and fails.

use std::{
    io::{self, Write},
    process,
    sync::atomic::{AtomicI32, Ordering},
};

use serde::Serialize;

use super::{
    cli_exit::CliExit, error::CliError, output, presenter::Presenter, report::print_error,
};

/// The code the command ends with if a write fails, set before a failed
/// outcome is written so a closed reader cannot turn it into success.
static EXIT_CODE: AtomicI32 = AtomicI32::new(0);

/// Sets the code the command ends with, before what it reports is written.
pub fn set_exit_code(code: CliExit) {
    EXIT_CODE.store(code.code(), Ordering::Relaxed);
}

/// The code set by [`set_exit_code`], success by default.
fn exit_code() -> i32 {
    EXIT_CODE.load(Ordering::Relaxed)
}

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

/// Ends the command after its output could not be written: quietly with the
/// code already set when the reader closed the pipe, with a failure
/// otherwise.
fn end_on_write_error(error: io::Error) -> ! {
    if error.kind() == io::ErrorKind::BrokenPipe {
        process::exit(exit_code());
    }
    print_error(&CliError::Output(error));
    let set = exit_code();
    process::exit(if set == CliExit::Success.code() {
        CliExit::ActionFailed.code()
    } else {
        set
    });
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

/// Prints `value` as [`print_result`] does and, when the command `failed`,
/// ends it with a failed action's code. Every command that can fail ends
/// here: the code is set before anything is written, so a reader that went
/// away or a full stream cannot turn the failure into success.
pub fn finish_result<T: Serialize>(json: bool, failed: bool, value: &T, human: impl FnOnce(&T)) {
    if failed {
        set_exit_code(CliExit::ActionFailed);
    }
    print_result(json, value, human);
    if failed {
        process::exit(CliExit::ActionFailed.code());
    }
}

/// Reports `error` as JSON on stdout with `json`, in the CLI's voice on
/// stderr otherwise, and exits with the code its kind documents.
pub fn fail_command(json: bool, error: &CliError) -> ! {
    fail_framed(Presenter::Plain, json, error)
}

/// Reports `error` as JSON on stdout with `json`, otherwise through
/// `presenter`, which closes a frame it opened, and exits with the code its
/// kind documents, set before anything is written.
pub fn fail_framed(presenter: Presenter, json: bool, error: &CliError) -> ! {
    if let CliError::Output(source) = error
        && source.kind() == io::ErrorKind::BrokenPipe
    {
        process::exit(exit_code());
    }
    let code = CliExit::from(error.kind());
    set_exit_code(code);
    if json {
        emit_line(&output::failure_json(error));
    } else {
        written(presenter.fail(error));
    }
    process::exit(code.code());
}
