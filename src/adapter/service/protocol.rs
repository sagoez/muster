//! Where the service listens and how a client talks to it: one JSON-line
//! [`ServiceRequest`] per connection, answered by one JSON-line
//! [`ServiceResponse`].

use std::{
    ffi::OsString,
    fs::{self, File},
    io::{self, Read, Write},
    os::unix::{
        ffi::{OsStrExt, OsStringExt},
        fs::{MetadataExt, OpenOptionsExt as _},
    },
    path::{Path, PathBuf},
    time::Duration,
};

use atomic_write_file::{AtomicWriteFile, unix::OpenOptionsExt as _};
use rustix::{
    fs::{Mode, OFlags, open},
    process::geteuid,
};
use uuid::Uuid;

use super::{service_request::ServiceRequest, service_response::ServiceResponse};
use crate::adapter::{holder::fnv1a, ipc, private_dir::GROUP_OTHER_BITS};

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
/// File in the state directory naming the socket the service bound.
const PUBLISHED_SOCKET_FILE: &str = "service.socket";
/// Mode of the published socket file: its owner's alone.
const PUBLISHED_SOCKET_MODE: u32 = 0o600;
/// Longest published socket path read, past any socket address.
const MAX_PUBLISHED_BYTES: u64 = 4096;

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

/// Records in `state_dir` that the service listens at `socket`, as an
/// owner-only file replaced atomically, so every client of that state
/// directory finds it whatever runtime directory it was started with.
///
/// # Errors
/// Returns the I/O error from writing the file.
pub fn publish_socket(state_dir: &Path, socket: &Path) -> io::Result<()> {
    let mut file = AtomicWriteFile::options()
        .mode(PUBLISHED_SOCKET_MODE)
        .preserve_mode(false)
        .open(state_dir.join(PUBLISHED_SOCKET_FILE))?;
    file.write_all(socket.as_os_str().as_bytes())?;
    file.commit()
}

/// The socket the service of `state_dir` published, when the file is a
/// regular owner-only file of this user naming an absolute path. Anything
/// else reads as none published.
pub fn published_socket(state_dir: &Path) -> Option<PathBuf> {
    let fd = open(
        state_dir.join(PUBLISHED_SOCKET_FILE),
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::NOCTTY | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .ok()?;
    let file = File::from(fd);
    let metadata = file.metadata().ok()?;
    let trusted = metadata.is_file()
        && metadata.uid() == geteuid().as_raw()
        && metadata.mode() & GROUP_OTHER_BITS == 0
        && metadata.len() <= MAX_PUBLISHED_BYTES;
    if !trusted {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(MAX_PUBLISHED_BYTES)
        .read_to_end(&mut bytes)
        .ok()?;
    Some(PathBuf::from(OsString::from_vec(bytes))).filter(|socket| socket.is_absolute())
}

/// Removes the record that the service listens at `socket`, unless it
/// already names another socket.
pub fn withdraw_socket(state_dir: &Path, socket: &Path) {
    if published_socket(state_dir).as_deref() == Some(socket) {
        let _ = fs::remove_file(state_dir.join(PUBLISHED_SOCKET_FILE));
    }
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
    use std::{
        fs::Permissions,
        os::unix::fs::{PermissionsExt, symlink},
    };

    use tempfile::TempDir;

    use super::*;

    /// A file mode other users can read.
    const SHARED_FILE_MODE: u32 = 0o644;

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

    /// A published socket reads back from an owner-only file, a withdrawal
    /// naming another socket leaves it, and its own removes it.
    #[test]
    fn a_published_socket_reads_back_until_withdrawn() {
        let state = TempDir::new().unwrap();
        let socket = Path::new("/run/user/1000/muster/0123456789abcdef.sock");

        assert_eq!(published_socket(state.path()), None);
        publish_socket(state.path(), socket).unwrap();

        assert_eq!(published_socket(state.path()).as_deref(), Some(socket));
        let mode = fs::metadata(state.path().join(PUBLISHED_SOCKET_FILE))
            .unwrap()
            .mode();
        assert_eq!(mode & GROUP_OTHER_BITS, 0);
        withdraw_socket(state.path(), Path::new("/elsewhere.sock"));
        assert_eq!(published_socket(state.path()).as_deref(), Some(socket));
        withdraw_socket(state.path(), socket);
        assert_eq!(published_socket(state.path()), None);
    }

    /// A published file others may read, a link, or a relative path reads as
    /// none published.
    #[test]
    fn an_untrusted_published_socket_is_ignored() {
        let state = TempDir::new().unwrap();
        let file = state.path().join(PUBLISHED_SOCKET_FILE);

        fs::write(&file, "/run/user/1000/muster/a.sock").unwrap();
        fs::set_permissions(&file, Permissions::from_mode(SHARED_FILE_MODE)).unwrap();
        assert_eq!(published_socket(state.path()), None);

        fs::remove_file(&file).unwrap();
        let target = state.path().join("target");
        publish_socket(state.path(), Path::new("/a.sock")).unwrap();
        fs::rename(&file, &target).unwrap();
        symlink(&target, &file).unwrap();
        assert_eq!(published_socket(state.path()), None);

        fs::remove_file(&file).unwrap();
        publish_socket(state.path(), Path::new("relative.sock")).unwrap();
        assert_eq!(published_socket(state.path()), None);
    }
}
