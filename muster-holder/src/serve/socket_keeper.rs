use std::{
    fs::{self, Metadata, Permissions},
    io,
    os::unix::{
        fs::{MetadataExt, PermissionsExt},
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

use muster_protocol::{
    holder::{HolderError, ensure_private_dir},
    ipc,
};

use crate::serve::socket_check::SocketCheck;

/// Mode of the socket: owner read and write only.
const PRIVATE_SOCKET_MODE: u32 = 0o600;

/// The holder's socket: its path, which file the holder bound there, and the
/// flag that stops its acceptors. The holder loop owns it, looks at the path
/// now and then, and binds it again when something removed it. Dropping it
/// stops the acceptor and removes the socket, only ever one the holder bound.
pub(super) struct SocketKeeper {
    /// Where the socket lives.
    path: PathBuf,
    /// The device and inode of the socket the holder last bound there.
    bound: (u64, u64),
    /// Set once the holder stops serving, so its acceptor stops.
    closing: Arc<AtomicBool>,
    /// The next connection's number, shared by every acceptor of the socket.
    connection_numbers: Arc<AtomicU64>,
    /// The trouble last reported, so trouble that lasts is reported once.
    reported: Option<String>,
}

impl SocketKeeper {
    /// Binds the socket at `path` so only its owner can reach it: its folder
    /// and the folder holding that are created owner-only and refused unless
    /// private, and the socket itself is owner read and write only. A stale
    /// socket nothing accepts on is replaced.
    ///
    /// # Errors
    /// Returns [`HolderError::InsecureSocketDir`] when a folder is not
    /// private or the path has none, or the I/O error from creating or
    /// binding it.
    pub(super) fn bind(path: &Path) -> Result<(Self, UnixListener), HolderError> {
        prepare_folders(path)?;
        let listener = ipc::bind(path)?;
        let bound = restrict(path)?;
        let keeper = Self {
            path: path.to_path_buf(),
            bound,
            closing: Arc::new(AtomicBool::new(false)),
            connection_numbers: Arc::new(AtomicU64::new(0)),
            reported: None,
        };
        Ok((keeper, listener))
    }

    /// The flag every acceptor of this socket stops on.
    pub(super) fn closing(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.closing)
    }

    /// The counter every acceptor of this socket numbers its connections
    /// from, so no two connections of the holder share a number.
    pub(super) fn connection_numbers(&self) -> Arc<AtomicU64> {
        Arc::clone(&self.connection_numbers)
    }

    /// Where the socket lives.
    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    /// Looks at the socket path. The socket the holder bound still being
    /// there is the usual case. A path where nothing is, as after a tmp
    /// cleaner ran, is bound again, its folders made and checked as at the
    /// first bind. Anything else at the path, a file or a socket another
    /// process bound, is never taken.
    pub(super) fn check(&mut self) -> SocketCheck {
        match fs::symlink_metadata(&self.path) {
            Ok(metadata) if identity(&metadata) == self.bound => SocketCheck::Intact,
            Ok(_) => SocketCheck::Occupied,
            Err(error) if error.kind() == io::ErrorKind::NotFound => self.rebind(),
            Err(error) => SocketCheck::Refused(error.into()),
        }
    }

    /// Binds the socket again at a path where nothing is. Binding fails
    /// rather than replace whatever appeared there meanwhile.
    fn rebind(&mut self) -> SocketCheck {
        let bound = prepare_folders(&self.path).and_then(|()| {
            let listener = UnixListener::bind(&self.path)?;
            Ok((restrict(&self.path)?, listener))
        });
        match bound {
            Ok((bound, listener)) => {
                self.bound = bound;
                SocketCheck::Rebound(listener)
            },
            Err(HolderError::Io(error)) if error.kind() == io::ErrorKind::AddrInUse => {
                SocketCheck::Occupied
            },
            Err(error) => SocketCheck::Refused(error),
        }
    }

    /// Whether `trouble` is news: anything but the trouble last reported.
    /// It is remembered, so trouble that lasts across looks is reported once.
    pub(super) fn is_news(&mut self, trouble: &str) -> bool {
        if self.reported.as_deref() == Some(trouble) {
            return false;
        }
        self.reported = Some(trouble.to_string());
        true
    }

    /// Forgets the trouble last reported, once the socket is served again.
    pub(super) fn clear_trouble(&mut self) {
        self.reported = None;
    }

    /// Whether the socket at the path is the one the holder last bound.
    fn is_own(&self) -> bool {
        fs::symlink_metadata(&self.path).is_ok_and(|metadata| identity(&metadata) == self.bound)
    }
}

impl Drop for SocketKeeper {
    /// Flags the acceptor, wakes it with a throwaway connection and unlinks the
    /// socket so no client can reach a holder that has stopped serving. What
    /// another process put at the path is left alone.
    fn drop(&mut self) {
        self.closing.store(true, Ordering::SeqCst);
        if self.is_own() {
            let _ = UnixStream::connect(&self.path);
            ipc::unlink(&self.path);
        }
    }
}

/// Creates the folder of the socket at `path`, and the folder holding that,
/// owner-only, refusing either unless it is a real directory this user owns
/// that no other user can enter.
///
/// # Errors
/// Returns [`HolderError::InsecureSocketDir`] when a folder is not private
/// or the path has none, or the I/O error from creating one.
fn prepare_folders(path: &Path) -> Result<(), HolderError> {
    let nonempty = |dir: &&Path| !dir.as_os_str().is_empty();
    let folder = path
        .parent()
        .filter(nonempty)
        .ok_or_else(|| HolderError::InsecureSocketDir(path.to_path_buf()))?;
    if let Some(root) = folder.parent().filter(nonempty) {
        ensure_private_dir(root)?;
    }
    ensure_private_dir(folder)
}

/// Makes the socket just bound at `path` owner read and write only, and
/// returns its device and inode. A socket that cannot be is unlinked.
///
/// # Errors
/// Returns the I/O error from changing or reading its mode.
fn restrict(path: &Path) -> Result<(u64, u64), HolderError> {
    let restricted = fs::set_permissions(path, Permissions::from_mode(PRIVATE_SOCKET_MODE))
        .and_then(|()| fs::symlink_metadata(path));
    match restricted {
        Ok(metadata) => Ok(identity(&metadata)),
        Err(error) => {
            ipc::unlink(path);
            Err(error.into())
        },
    }
}

/// The device and inode of a file, which tell one file from another put at
/// the same path.
fn identity(metadata: &Metadata) -> (u64, u64) {
    (metadata.dev(), metadata.ino())
}
