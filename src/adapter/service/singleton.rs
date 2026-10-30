use std::{
    fs::{File, OpenOptions, TryLockError},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};

use getset::Getters;

use super::service_error::ServiceError;
use crate::adapter::private_dir::prepare_state_dir;

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
            Err(TryLockError::Error(error)) => Err(error.into()),
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
}
