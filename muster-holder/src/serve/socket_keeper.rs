use std::{
    fs::{self, Metadata, Permissions},
    io,
    os::unix::{
        fs::{FileTypeExt, MetadataExt, PermissionsExt},
        net::UnixListener,
    },
    path::{Path, PathBuf},
    sync::Arc,
};

use crossbeam_channel::Sender;
use muster_protocol::{
    holder::{HolderError, SocketDirPlace, check_socket_folder, ensure_socket_folder, folder_of},
    ipc,
};

use crate::serve::{
    acceptor::Acceptor, acceptor_counts::AcceptorCounts, holder_event::HolderEvent,
    socket_check::SocketCheck, socket_trouble::SocketTrouble,
};

/// Mode of the socket: owner read and write only.
const PRIVATE_SOCKET_MODE: u32 = 0o600;

/// What tells the socket the holder bound from any file put at its path
/// later: its device and inode. The holder's listener keeps the socket's
/// inode from being freed while it is open, so no other file can take its
/// number, and touching, moving or linking the socket keeps it.
type SocketIdentity = (u64, u64);

/// The holder's socket and the thread accepting on it. The holder loop owns
/// it, looks at the path now and then, and binds it again when something
/// removed it or its acceptor ended, letting the old acceptor go so only
/// one ever serves. Dropping it lets the acceptor go and removes the
/// socket, only ever one the holder bound.
pub(super) struct SocketKeeper {
    /// Where the socket lives.
    path: PathBuf,
    /// The socket the holder last served there.
    bound: SocketIdentity,
    /// The acceptor of the socket the holder last served, while it has one.
    acceptor: Option<Acceptor>,
    /// Feeds accepted connections to the holder loop.
    events: Sender<HolderEvent>,
    /// What every acceptor of the socket shares.
    counts: Arc<AcceptorCounts>,
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
    /// Returns [`HolderError::SocketWithoutFolder`] when the path names no
    /// folder, [`HolderError::InsecureSocketDir`] when a folder is not
    /// private, [`HolderError::SocketReplaced`] when the path stops holding
    /// the socket, or the I/O error from creating, binding or serving it.
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
            counts: Arc::default(),
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

    /// What every acceptor of the socket shares, counted.
    #[cfg(test)]
    pub(super) fn counts(&self) -> &AcceptorCounts {
        &self.counts
    }

    /// The listener the current acceptor serves, while there is one, which
    /// only a Linux test shuts down.
    #[cfg(all(test, target_os = "linux"))]
    pub(super) fn listener(&self) -> Option<&UnixListener> {
        self.acceptor.as_ref().map(Acceptor::listener)
    }

    /// Looks at the socket path. The socket the holder bound still being
    /// there and served is the usual case. A path where nothing is, as after
    /// a tmp cleaner ran, is bound again, its folders made and checked as at
    /// the first bind, and so is the holder's own socket once nothing accepts
    /// on it. Anything else at the path, a file or a socket another process
    /// bound, even one that has since died, is never taken; it is bound
    /// again only once that file is removed.
    pub(super) fn check(&mut self) -> SocketCheck {
        match fs::symlink_metadata(&self.path) {
            Ok(metadata) if is_own(&metadata, self.bound) && self.is_served() => {
                SocketCheck::Intact
            },
            Ok(metadata) if is_own(&metadata, self.bound) => {
                self.acceptor = None;
                ipc::unlink(&self.path);
                self.rebind()
            },
            Ok(_) => SocketCheck::Occupied,
            Err(error) if error.kind() == io::ErrorKind::NotFound => self.rebind(),
            Err(error) => SocketCheck::Refused(error.into()),
        }
    }

    /// Whether an acceptor still accepts on the socket.
    fn is_served(&self) -> bool {
        self.acceptor
            .as_ref()
            .is_some_and(|acceptor| !acceptor.has_ended())
    }

    /// Binds the socket again at a path where nothing is, and moves accepting
    /// onto it. Binding fails rather than replace whatever appeared there
    /// meanwhile. The new socket counts as the holder's only once it is
    /// served; one no acceptor could start on is unlinked, so the next look
    /// binds again.
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
        match self.accept_on(listener) {
            Ok(()) => {
                self.bound = bound;
                SocketCheck::Rebound
            },
            Err(error) => {
                if fs::symlink_metadata(&self.path).is_ok_and(|metadata| is_own(&metadata, bound)) {
                    ipc::unlink(&self.path);
                }
                SocketCheck::Refused(error)
            },
        }
    }

    /// Accepts on `listener` from a fresh acceptor, then lets the one of the
    /// socket served before go, which stops without the loop waiting for it.
    ///
    /// # Errors
    /// Returns the I/O error from starting the new acceptor; the old one is
    /// kept then.
    fn accept_on(&mut self, listener: UnixListener) -> Result<(), HolderError> {
        let acceptor = Acceptor::start(listener, self.events.clone(), &self.counts)?;
        self.acceptor = Some(acceptor);
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

    /// Whether the socket at the path is the one the holder last served.
    fn is_own(&self) -> bool {
        fs::symlink_metadata(&self.path).is_ok_and(|metadata| is_own(&metadata, self.bound))
    }
}

impl Drop for SocketKeeper {
    /// Lets the acceptor go, which closes its listener as its thread
    /// returns, and unlinks the socket so no client can reach a holder that
    /// has stopped serving. What another process put at the path is left
    /// alone.
    fn drop(&mut self) {
        if self.is_own() {
            ipc::unlink(&self.path);
        }
        self.acceptor = None;
    }
}

/// The folder of the socket at `path`.
///
/// # Errors
/// Returns [`HolderError::SocketWithoutFolder`] when the path names none.
fn socket_folder(path: &Path) -> Result<&Path, HolderError> {
    folder_of(path).ok_or_else(|| HolderError::SocketWithoutFolder(path.to_path_buf()))
}

/// Makes the socket just bound at `path` owner read and write only, once
/// sure it is still a socket in a private `folder`, and returns its
/// identity. A socket that cannot be is unlinked; anything else found at the
/// path is left.
///
/// # Errors
/// Returns [`HolderError::SocketReplaced`] when the path stopped holding a
/// socket, [`HolderError::InsecureSocketDir`] when the folder stopped being
/// private, or the I/O error from changing or reading its mode.
fn restrict(path: &Path, folder: &Path) -> Result<SocketIdentity, HolderError> {
    let replaced = || HolderError::SocketReplaced(path.to_path_buf());
    if !fs::symlink_metadata(path)?.file_type().is_socket() {
        return Err(replaced());
    }
    let restricted = check_socket_folder(folder, SocketDirPlace::Tmp)
        .and_then(|()| {
            Ok(fs::set_permissions(
                path,
                Permissions::from_mode(PRIVATE_SOCKET_MODE),
            )?)
        })
        .and_then(|()| Ok(fs::symlink_metadata(path)?));
    match restricted {
        Ok(metadata) if metadata.file_type().is_socket() => Ok(identity(&metadata)),
        Ok(_) => Err(replaced()),
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

/// The identity of a file: its device and inode.
fn identity(metadata: &Metadata) -> SocketIdentity {
    (metadata.dev(), metadata.ino())
}
