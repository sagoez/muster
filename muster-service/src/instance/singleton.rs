use std::{
    fs::{File, OpenOptions, TryLockError},
    io,
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};

use getset::Getters;
use muster_os::private_dir::prepare_state_dir;

use crate::service_error::ServiceError;

/// File in the state directory whose lock marks the running service.
const LOCK_FILE: &str = "service.lock";
/// Mode of the lock file: the owner's alone.
const LOCK_FILE_MODE: u32 = 0o600;

/// Proof that this process is the one service of its state directory: an
/// exclusive lock on `service.lock`, held until it is dropped or the process
/// ends, so a crashed service never leaves a stale claim behind.
#[derive(Debug, Getters)]
pub struct Singleton {
    /// The locked file; closing it releases the lock.
    lock: File,
    /// The state directory this service owns.
    #[getset(get = "pub")]
    state_dir: PathBuf,
}

impl Singleton {
    /// Takes the service lock in `state_dir`, creating the directory
    /// owner-only and the lock file as needed. Returns `None` while another
    /// service holds it.
    ///
    /// # Errors
    /// Returns a [`ServiceError`] when the directory or lock file cannot be
    /// created or the lock cannot be attempted.
    pub fn acquire(state_dir: &Path) -> Result<Option<Singleton>, ServiceError> {
        prepare_state_dir(state_dir)?;
        Ok(Self::acquire_existing(state_dir)?)
    }

    /// Takes the service lock in `state_dir` as [`Self::acquire`] does, but
    /// never creates the directory: one moved away meanwhile is not made
    /// again where it was.
    ///
    /// # Errors
    /// Returns the I/O error from opening the lock file, of kind
    /// [`io::ErrorKind::NotFound`] when the directory is gone, or from
    /// attempting the lock.
    pub fn acquire_existing(state_dir: &Path) -> io::Result<Option<Singleton>> {
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .mode(LOCK_FILE_MODE)
            .open(state_dir.join(LOCK_FILE))?;
        match lock.try_lock() {
            Ok(()) => Ok(Some(Self {
                lock,
                state_dir: state_dir.to_path_buf(),
            })),
            Err(TryLockError::WouldBlock) => Ok(None),
            Err(TryLockError::Error(error)) => Err(error),
        }
    }
}

impl Drop for Singleton {
    /// Releases the lock at once, so a successor can take it before this
    /// process has fully exited.
    fn drop(&mut self) {
        let _ = self.lock.unlock();
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn second_singleton_acquire_returns_none_while_first_is_held() {
        let scratch = TempDir::new().unwrap();

        let first = Singleton::acquire(scratch.path()).unwrap();
        assert!(first.is_some());
        assert!(Singleton::acquire(scratch.path()).unwrap().is_none());

        drop(first);
        assert!(Singleton::acquire(scratch.path()).unwrap().is_some());
    }

    /// A lock taken only in an existing directory fails for one moved away,
    /// and never makes it again.
    #[test]
    fn an_existing_only_lock_never_makes_its_directory() {
        let scratch = TempDir::new().unwrap();
        let gone = scratch.path().join("moved");

        let error = Singleton::acquire_existing(&gone).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::NotFound);
        assert!(!gone.exists());
        assert!(
            Singleton::acquire_existing(scratch.path())
                .unwrap()
                .is_some()
        );
    }
}
