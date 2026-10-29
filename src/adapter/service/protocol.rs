//! Where the service listens and how a client talks to it: one JSON-line
//! [`ServiceRequest`] per connection, answered by one JSON-line
//! [`ServiceResponse`].

use std::{
    io,
    path::{Path, PathBuf},
    time::Duration,
};

use super::{service_request::ServiceRequest, service_response::ServiceResponse};
use crate::adapter::ipc;

/// Hidden subcommand that runs the service in the current process.
pub const SERVICE_SUBCOMMAND: &str = "__service";
/// File name of the service socket in the state directory.
const SOCKET_FILE: &str = "service.sock";
/// File name of the log the service mirrors its warnings and errors to.
const LOG_FILE: &str = "service.log";

/// The socket the service of `state_dir` listens on.
pub fn socket_path(state_dir: &Path) -> PathBuf {
    state_dir.join(SOCKET_FILE)
}

/// The log the service of `state_dir` mirrors its notices to.
pub fn log_path(state_dir: &Path) -> PathBuf {
    state_dir.join(LOG_FILE)
}

/// Sends `request` to the service at `socket` and returns its answer.
///
/// # Errors
/// Returns an [`io::Error`] when no service accepts at `socket` or the
/// exchange fails.
pub fn send(socket: &Path, request: &ServiceRequest) -> io::Result<ServiceResponse> {
    ipc::request(socket, request)
}

/// As [`send`], giving up when the answer takes longer than `reply_timeout`.
///
/// # Errors
/// Returns an [`io::Error`] when no service accepts at `socket`, the exchange
/// fails, or the answer is late.
pub fn send_within(
    socket: &Path,
    request: &ServiceRequest,
    reply_timeout: Duration,
) -> io::Result<ServiceResponse> {
    ipc::request_within(socket, request, reply_timeout)
}
