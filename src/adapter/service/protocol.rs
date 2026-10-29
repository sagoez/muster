//! Where the service listens and how a client talks to it: one JSON-line
//! [`ServiceRequest`] per connection, answered by one JSON-line
//! [`ServiceResponse`].

use std::{
    io,
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
    time::Duration,
};

use uuid::Uuid;

use super::{service_request::ServiceRequest, service_response::ServiceResponse};
use crate::adapter::{holder::fnv1a, ipc};

/// Hidden subcommand that runs the service in the current process.
pub const SERVICE_SUBCOMMAND: &str = "__service";
/// Extension of the service socket's file name.
const SOCKET_EXTENSION: &str = "sock";
/// Prefix of the name the service first binds its socket at.
const STAGING_PREFIX: &str = ".";
/// Hex digits of the random name the service first binds its socket at.
const STAGING_DIGITS: usize = 8;
/// File name of the log the service mirrors its warnings and errors to.
const LOG_FILE: &str = "service.log";

/// The socket the service of `state_dir` listens on, in `socket_dir`. It is
/// named for a stable hash of the state directory, so services of two state
/// directories sharing one runtime directory never take each other's.
pub fn socket_path(socket_dir: &Path, state_dir: &Path) -> PathBuf {
    socket_dir
        .join(format!("{:016x}", fnv1a(state_dir.as_os_str().as_bytes())))
        .with_extension(SOCKET_EXTENSION)
}

/// A fresh name beside `socket` for the service to bind first, then make
/// private and move into place. It is never longer than the socket's own.
pub fn staging_socket(socket: &Path) -> PathBuf {
    let nonce = Uuid::new_v4().simple().to_string();
    socket.with_file_name(format!("{STAGING_PREFIX}{}", &nonce[..STAGING_DIGITS]))
}

/// The log the service of `state_dir` mirrors its notices to.
pub fn log_path(state_dir: &Path) -> PathBuf {
    state_dir.join(LOG_FILE)
}

/// Sends `request` to the service at `socket` and returns its answer, giving
/// up when it takes longer than `reply_timeout`.
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Each state directory has its own socket in a shared socket directory,
    /// the same one every time, and staging names stay beside it and no
    /// longer.
    #[test]
    fn each_state_dir_has_its_own_short_socket() {
        let dir = Path::new("/run/user/1000/muster");
        let socket = socket_path(dir, Path::new("/home/sam/.local/state/muster"));

        assert_eq!(
            socket,
            socket_path(dir, Path::new("/home/sam/.local/state/muster"))
        );
        assert_ne!(socket, socket_path(dir, Path::new("/tmp/elsewhere/muster")));
        assert_eq!(socket.parent(), Some(dir));
        let staged = staging_socket(&socket);
        assert_eq!(staged.parent(), Some(dir));
        assert_ne!(staged, staging_socket(&socket));
        assert!(staged.as_os_str().len() <= socket.as_os_str().len());
    }
}
