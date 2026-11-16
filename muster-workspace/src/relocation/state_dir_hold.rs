use std::{io, path::PathBuf};

use muster_os::{no_state_dir::NoStateDir, state_location::StateLocation};
use muster_service::{ServiceError, Singleton};

/// Most times a starting service resolves its state directory again because
/// a move took it away while it took its lock.
const HOLD_ATTEMPTS: usize = 3;
/// Why a service gave up holding its state directory.
const KEPT_MOVING: &str = "the state directory kept moving while the service took its lock";

/// The state directory a starting service runs from and its lock there, or
/// none when another service holds it. `resolve` says where the state is;
/// it is asked again once the lock is held, and a directory that moved
/// meanwhile is let go and the new one taken, so a service never runs from
/// a place a move made a link. An old directory still waiting to move is
/// locked where it is and never made again once gone.
///
/// # Errors
/// Returns a [`ServiceError`] when there is no state directory, it or its
/// lock cannot be used, or it moved on every attempt.
pub fn hold_state_dir(
    resolve: &dyn Fn() -> Result<StateLocation, NoStateDir>,
) -> Result<(PathBuf, Option<Singleton>), ServiceError> {
    for _ in 0..HOLD_ATTEMPTS {
        let location = resolve()?;
        let dir = location.in_use().to_path_buf();
        let lock = if location.pending_move().is_some() {
            match Singleton::acquire_existing(&dir) {
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                held => held?,
            }
        } else {
            Singleton::acquire(&dir)?
        };
        let Some(lock) = lock else {
            return Ok((dir, None));
        };
        if resolve()?.in_use() == dir {
            return Ok((dir, Some(lock)));
        }
    }
    Err(io::Error::other(KEPT_MOVING).into())
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, fs, os::unix::fs::symlink};

    use super::*;
    use crate::relocation::legacy_home::LegacyHome;

    /// A service that locked the old directory just as a move took it away
    /// lets it go and runs from the new place, never from the link.
    #[test]
    fn a_service_whose_dir_moves_while_it_locks_runs_from_the_new_place() {
        let legacy = LegacyHome::new();
        let asked = Cell::new(0);
        let resolve = || {
            asked.set(asked.get() + 1);
            if asked.get() == 2 {
                fs::create_dir_all(legacy.to().parent().unwrap()).unwrap();
                fs::rename(legacy.from(), legacy.to()).unwrap();
                symlink(legacy.to(), legacy.from()).unwrap();
            }
            Ok(legacy.location())
        };

        let (dir, lock) = hold_state_dir(&resolve).unwrap();

        assert_eq!(dir, legacy.to());
        assert!(lock.is_some());
    }

    /// An old directory still waiting to move is the one a service runs
    /// from, and another service's lock there is respected.
    #[test]
    fn a_service_runs_from_the_legacy_dir_while_it_waits_to_move() {
        let legacy = LegacyHome::new();
        let resolve = || Ok(legacy.location());

        let (dir, lock) = hold_state_dir(&resolve).unwrap();
        assert_eq!(dir, legacy.from());
        assert!(lock.is_some());

        let (_, second) = hold_state_dir(&resolve).unwrap();
        assert!(second.is_none(), "two services held one directory");
        assert!(!legacy.to().exists());
    }
}
