use std::{
    fs::{File, OpenOptions, TryLockError},
    io,
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

use getset::Getters;
use muster_os::{
    no_state_dir::NoStateDir,
    private_dir::{check_state_dir, prepare_state_dir},
    state_location::StateLocation,
};
use rustix::process::geteuid;

use crate::service_error::ServiceError;

/// Most times a starting service resolves its state directory again
/// because a move took it away while it took its lock.
const HOLD_ATTEMPTS: usize = 3;
/// How long a starting service keeps trying a lock it finds held, which a
/// command moving the state holds for a moment, before taking the holder
/// for another service.
pub const HOLD_WAIT: Duration = Duration::from_millis(250);
/// Pause between those tries.
const HOLD_POLL: Duration = Duration::from_millis(10);
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
        prepare_state_dir(state_dir, None)?;
        Ok(Self::acquire_existing(state_dir)?)
    }

    /// The state directory a starting service runs from and its lock there,
    /// or none when another service holds it. `resolve` says where the
    /// state is; it is asked again once the lock is held, and a directory
    /// that moved meanwhile is let go and the new one taken, so a service
    /// never runs from a place a move made a link. An old directory still
    /// waiting to move is checked as this user's and locked where it is,
    /// never made again once gone, and the new one is never made beside it.
    /// A lock found held is tried again for [`HOLD_WAIT`], since a command
    /// moving the state holds it for a moment.
    ///
    /// # Errors
    /// Returns a [`ServiceError`] when there is no state directory, it or
    /// its lock cannot be used, or it moved on every attempt.
    pub fn hold(
        resolve: &dyn Fn() -> Result<StateLocation, NoStateDir>,
    ) -> Result<(PathBuf, Option<Singleton>), ServiceError> {
        let deadline = Instant::now() + HOLD_WAIT;
        let mut moves = 0;
        while moves < HOLD_ATTEMPTS {
            let location = resolve()?;
            let dir = location.in_use();
            let ready = if location.pending_move().is_some() {
                check_state_dir(&dir, geteuid().as_raw())
            } else {
                prepare_state_dir(&dir, location.legacy().as_deref())
            };
            match ready.and_then(|()| Self::acquire_existing(&dir)) {
                Err(error) if error.kind() == io::ErrorKind::NotFound => moves += 1,
                Err(error) => return Err(error.into()),
                Ok(None) if Instant::now() < deadline => thread::sleep(HOLD_POLL),
                Ok(None) => return Ok((dir, None)),
                Ok(Some(lock)) if resolve()?.in_use() == dir => return Ok((dir, Some(lock))),
                Ok(Some(_)) => moves += 1,
            }
        }
        Err(ServiceError::StateDirMoving)
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
    use std::{
        cell::Cell,
        fs::{self, Permissions},
        os::unix::fs::{PermissionsExt, symlink},
    };

    use tempfile::TempDir;

    use super::*;

    /// Where an earlier muster kept its state, in a scratch home.
    const LEGACY_DIR: &str = "old/muster";
    /// The state directory it moves to.
    const STATE_DIR: &str = "new/state/muster";
    /// A folder mode every user may write, not sticky.
    const OPEN_DIR_MODE: u32 = 0o777;

    /// A scratch home with an earlier muster's state directory in it, and
    /// the location whose legacy directory that is.
    fn legacy_home() -> (TempDir, StateLocation) {
        let home = TempDir::new().unwrap();
        fs::create_dir_all(home.path().join(LEGACY_DIR)).unwrap();
        let location = StateLocation::builder()
            .dir(home.path().join(STATE_DIR))
            .legacy(Some(home.path().join(LEGACY_DIR)))
            .build();
        (home, location)
    }

    /// A service that locked the old directory just as a move took it away
    /// lets it go and runs from the new place, never from the link.
    #[test]
    fn a_service_whose_dir_moves_while_it_locks_runs_from_the_new_place() {
        let (home, location) = legacy_home();
        let (from, to) = (home.path().join(LEGACY_DIR), home.path().join(STATE_DIR));
        let asked = Cell::new(0);
        let resolve = || {
            asked.set(asked.get() + 1);
            if asked.get() == 2 {
                fs::create_dir_all(to.parent().unwrap()).unwrap();
                fs::rename(&from, &to).unwrap();
                symlink(&to, &from).unwrap();
            }
            Ok(location.clone())
        };

        let (dir, lock) = Singleton::hold(&resolve).unwrap();

        assert_eq!(dir, to);
        assert!(lock.is_some());
    }

    /// An old directory still waiting to move is the one a service runs
    /// from, the new one is not made, and another service's lock there is
    /// respected once a mover would have let go of it.
    #[test]
    fn a_service_runs_from_the_legacy_dir_while_it_waits_to_move() {
        let (home, location) = legacy_home();
        let resolve = || Ok(location.clone());

        let (dir, lock) = Singleton::hold(&resolve).unwrap();
        assert_eq!(dir, home.path().join(LEGACY_DIR));
        assert!(lock.is_some());

        let started = Instant::now();
        let (_, second) = Singleton::hold(&resolve).unwrap();
        assert!(second.is_none(), "two services held one directory");
        assert!(started.elapsed() >= HOLD_WAIT, "a held lock was not waited");
        assert!(!home.path().join(STATE_DIR).exists());
    }

    /// An old directory that is not safely this user's, here in a folder
    /// every user may write, is refused before the service writes anything
    /// in it, its lock file included.
    #[test]
    fn a_legacy_dir_not_safely_this_users_is_refused_before_it_is_touched() {
        let (home, location) = legacy_home();
        let folder = home.path().join(LEGACY_DIR).parent().unwrap().to_path_buf();
        fs::set_permissions(&folder, Permissions::from_mode(OPEN_DIR_MODE)).unwrap();

        let error = Singleton::hold(&|| Ok(location.clone())).unwrap_err();

        assert!(error.to_string().contains("another user"), "{error}");
        assert!(!home.path().join(LEGACY_DIR).join(LOCK_FILE).exists());
    }

    /// A lock a mover holds for a moment is waited out, so a service started
    /// meanwhile still starts.
    #[test]
    fn a_lock_held_for_a_moment_is_waited_out() {
        let (home, location) = legacy_home();
        let held = Singleton::acquire_existing(&home.path().join(LEGACY_DIR)).unwrap();
        let release = thread::spawn(move || {
            thread::sleep(HOLD_POLL * 3);
            drop(held);
        });

        let (_, lock) = Singleton::hold(&|| Ok(location.clone())).unwrap();

        release.join().unwrap();
        assert!(lock.is_some(), "the service took the mover for a service");
    }

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
