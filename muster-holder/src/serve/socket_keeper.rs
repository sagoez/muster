#[cfg(test)]
use std::sync::atomic::Ordering;
use std::{
    fs::{self, Metadata, Permissions},
    io,
    os::unix::{
        fs::{FileTypeExt, MetadataExt, PermissionsExt},
        net::UnixListener,
    },
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, AtomicUsize},
    },
};

use crossbeam_channel::Sender;
use muster_protocol::{
    holder::{HolderError, check_socket_folder, ensure_socket_folder},
    ipc,
};

use crate::serve::{
    acceptor::Acceptor, holder_event::HolderEvent, socket_check::SocketCheck,
    socket_trouble::SocketTrouble,
};

/// Mode of the socket: owner read and write only.
const PRIVATE_SOCKET_MODE: u32 = 0o600;

/// What tells the socket the holder bound from any file put at its path
/// later: its device and inode, and the time its inode last changed, which a
/// new file reusing a freed inode number does not share.
type SocketIdentity = (u64, u64, i64, i64);

/// The holder's socket and the thread accepting on it. The holder loop owns
/// it, looks at the path now and then, and binds it again when something
/// removed it, stopping the old acceptor so only one ever runs. Dropping it
/// stops the acceptor and removes the socket, only ever one the holder
/// bound.
pub(super) struct SocketKeeper {
    /// Where the socket lives.
    path: PathBuf,
    /// The socket the holder last bound there.
    bound: SocketIdentity,
    /// The thread accepting on the socket the holder last bound.
    acceptor: Option<Acceptor>,
    /// Feeds accepted connections to the holder loop.
    events: Sender<HolderEvent>,
    /// The next connection's number, shared by every acceptor of the socket.
    connection_numbers: Arc<AtomicU64>,
    /// How many acceptors of the socket run.
    running_acceptors: Arc<AtomicUsize>,
    /// The trouble last warned of, so trouble that lasts is warned of once.
    warned: Option<SocketTrouble>,
    /// Looks in a row that found the socket intact.
    quiet_checks: u32,
}

impl SocketKeeper {
    /// Binds the socket at `path` so only its owner can reach it, and starts
    /// accepting on it, handing connections to `events`. Its folder and the
    /// folder holding that are created owner-only and refused unless private,
    /// and the socket itself is owner read and write only. A stale socket
    /// nothing accepts on is replaced.
    ///
    /// # Errors
    /// Returns [`HolderError::InsecureSocketDir`] when a folder is not
    /// private or the path has none, or the I/O error from creating or
    /// binding it.
    pub(super) fn bind(path: &Path, events: Sender<HolderEvent>) -> Result<Self, HolderError> {
        let folder = socket_folder(path)?;
        ensure_socket_folder(folder)?;
        let listener = ipc::bind(path)?;
        let bound = restrict(path, folder)?;
        let mut keeper = Self {
            path: path.to_path_buf(),
            bound,
            acceptor: None,
            events,
            connection_numbers: Arc::new(AtomicU64::new(0)),
            running_acceptors: Arc::new(AtomicUsize::new(0)),
            warned: None,
            quiet_checks: 0,
        };
        keeper.accept_on(listener)?;
        Ok(keeper)
    }

    /// Where the socket lives.
    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    /// How many acceptors of the socket run: one while the holder serves.
    #[cfg(test)]
    pub(super) fn running_acceptors(&self) -> usize {
        self.running_acceptors.load(Ordering::SeqCst)
    }

    /// Looks at the socket path. The socket the holder bound still being
    /// there is the usual case. A path where nothing is, as after a tmp
    /// cleaner ran, is bound again, its folders made and checked as at the
    /// first bind. Anything else at the path, a file or a socket another
    /// process bound, even one that has since died, is never taken; it is
    /// bound again only once that file is removed.
    pub(super) fn check(&mut self) -> SocketCheck {
        match fs::symlink_metadata(&self.path) {
            Ok(metadata) if is_own(&metadata, self.bound) => SocketCheck::Intact,
            Ok(_) => SocketCheck::Occupied,
            Err(error) if error.kind() == io::ErrorKind::NotFound => self.rebind(),
            Err(error) => SocketCheck::Refused(error.into()),
        }
    }

    /// Binds the socket again at a path where nothing is, and moves accepting
    /// onto it. Binding fails rather than replace whatever appeared there
    /// meanwhile.
    fn rebind(&mut self) -> SocketCheck {
        let bound = socket_folder(&self.path).and_then(|folder| {
            ensure_socket_folder(folder)?;
            let listener = UnixListener::bind(&self.path)?;
            Ok((restrict(&self.path, folder)?, listener))
        });
        let (bound, listener) = match bound {
            Ok(bound) => bound,
            Err(HolderError::Io(error)) if error.kind() == io::ErrorKind::AddrInUse => {
                return SocketCheck::Occupied;
            },
            Err(error) => return SocketCheck::Refused(error),
        };
        self.bound = bound;
        match self.accept_on(listener) {
            Ok(()) => SocketCheck::Rebound,
            Err(error) => SocketCheck::Refused(error),
        }
    }

    /// Stops the acceptor of the socket bound before, closing its listener,
    /// and accepts on `listener` instead.
    ///
    /// # Errors
    /// Returns the I/O error from starting the new acceptor.
    fn accept_on(&mut self, listener: UnixListener) -> Result<(), HolderError> {
        self.acceptor = None;
        self.acceptor = Some(Acceptor::start(
            listener,
            self.events.clone(),
            Arc::clone(&self.connection_numbers),
            Arc::clone(&self.running_acceptors),
        )?);
        Ok(())
    }

    /// Whether `trouble` is worth a warning: trouble of another kind than
    /// the last warned of, or any after `quiet` looks in a row found the
    /// socket intact. It is remembered, so trouble that lasts, or a socket
    /// removed over and over, is warned of once.
    pub(super) fn is_news(&mut self, trouble: SocketTrouble, quiet: u32) -> bool {
        let news = self.warned != Some(trouble) || self.quiet_checks >= quiet;
        self.warned = Some(trouble);
        self.quiet_checks = 0;
        news
    }

    /// Counts a look that found the socket intact.
    pub(super) fn count_quiet(&mut self) {
        self.quiet_checks = self.quiet_checks.saturating_add(1);
    }

    /// Whether the socket at the path is the one the holder last bound.
    fn is_own(&self) -> bool {
        fs::symlink_metadata(&self.path).is_ok_and(|metadata| is_own(&metadata, self.bound))
    }
}

impl Drop for SocketKeeper {
    /// Stops the acceptor, closing its listener, and unlinks the socket so no
    /// client can reach a holder that has stopped serving. What another
    /// process put at the path is left alone.
    fn drop(&mut self) {
        self.acceptor = None;
        if self.is_own() {
            ipc::unlink(&self.path);
        }
    }
}

/// The folder of the socket at `path`.
///
/// # Errors
/// Returns [`HolderError::InsecureSocketDir`] when the path has none.
fn socket_folder(path: &Path) -> Result<&Path, HolderError> {
    path.parent()
        .filter(|folder| !folder.as_os_str().is_empty())
        .ok_or_else(|| HolderError::InsecureSocketDir {
            dir: path.to_path_buf(),
            owner: None,
        })
}

/// Makes the socket just bound at `path` owner read and write only, once
/// sure it is still a socket in a private `folder`, and returns its
/// identity. A socket that cannot be is unlinked; anything else found at the
/// path is left.
///
/// # Errors
/// Returns [`HolderError::InsecureSocketDir`] when the folder stopped being
/// private or the path stopped holding a socket, or the I/O error from
/// changing or reading its mode.
fn restrict(path: &Path, folder: &Path) -> Result<SocketIdentity, HolderError> {
    let not_a_socket = || HolderError::InsecureSocketDir {
        dir: folder.to_path_buf(),
        owner: None,
    };
    if !fs::symlink_metadata(path)?.file_type().is_socket() {
        return Err(not_a_socket());
    }
    let restricted = check_socket_folder(folder)
        .and_then(|()| {
            Ok(fs::set_permissions(
                path,
                Permissions::from_mode(PRIVATE_SOCKET_MODE),
            )?)
        })
        .and_then(|()| Ok(fs::symlink_metadata(path)?));
    match restricted {
        Ok(metadata) if metadata.file_type().is_socket() => Ok(identity(&metadata)),
        Ok(_) => Err(not_a_socket()),
        Err(error) => {
            ipc::unlink(path);
            Err(error)
        },
    }
}

/// Whether `metadata` is of the socket with identity `bound`.
fn is_own(metadata: &Metadata, bound: SocketIdentity) -> bool {
    metadata.file_type().is_socket() && identity(metadata) == bound
}

/// The identity of a file: its device, inode and inode change time.
fn identity(metadata: &Metadata) -> SocketIdentity {
    (
        metadata.dev(),
        metadata.ino(),
        metadata.ctime(),
        metadata.ctime_nsec(),
    )
}
