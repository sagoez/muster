use std::{fs::OpenOptions, io, os::unix::fs::OpenOptionsExt, path::Path};

use rustix::stdio::dup2_stderr;

/// Mode of the service log: its owner's alone.
pub(super) const LOG_FILE_MODE: u32 = 0o600;

/// Sends everything this process writes to standard error to the end of
/// `log`, so a service started with no terminal still leaves its failures
/// where `service.log` readers look.
///
/// # Errors
/// Returns the I/O error from opening the log or redirecting the stream.
pub fn redirect_stderr(log: &Path) -> io::Result<()> {
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(LOG_FILE_MODE)
        .open(log)?;
    dup2_stderr(&file)?;
    Ok(())
}
