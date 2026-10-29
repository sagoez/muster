use std::{
    fs::OpenOptions,
    io,
    os::{fd::AsRawFd, unix::fs::OpenOptionsExt},
    path::Path,
};

/// Mode of the service log: its owner's alone.
const LOG_FILE_MODE: u32 = 0o600;
/// The process's standard error stream.
const STDERR_FD: i32 = 2;

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
    // SAFETY: both descriptors are valid for this call: `file` is open and
    // stderr always exists; `dup2` touches no memory.
    if unsafe { libc::dup2(file.as_raw_fd(), STDERR_FD) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
