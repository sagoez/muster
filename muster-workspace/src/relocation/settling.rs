//! Settling where muster's state is before a command or a service uses it:
//! the one-time move when it is due, else what still keeps the state apart.

use muster_os::state_location::StateLocation;
use muster_store::DATABASE_FILE;

use super::{
    legacy_state_move::LegacyStateMove,
    move_failure::{record_move_failure, recorded_move_failure},
    state_move_error::StateMoveError,
};

/// Moves the state directory an earlier muster kept elsewhere when that is
/// due, or finishes a move cut short; else reports what [`standing`] does.
///
/// # Errors
/// Returns a [`StateMoveError`] saying why the old state did not move.
pub fn settle(location: &StateLocation) -> Result<(), StateMoveError> {
    match LegacyStateMove::pending(location) {
        Some(state_move) => state_move.run(),
        None => standing(location),
    }
}

/// As [`settle`], for a starting service: when its own move fails, why is
/// recorded in the old directory it then runs from, so every command says
/// so rather than take it for an older service.
///
/// # Errors
/// Returns a [`StateMoveError`] saying why the old state did not move.
pub fn settle_as_service(location: &StateLocation) -> Result<(), StateMoveError> {
    let settled = settle(location);
    if let Err(error) = &settled
        && error.is_failed_move()
        && let Some(from) = location.pending_move()
    {
        record_move_failure(from, &error.to_string());
    }
    settled
}

/// What keeps the state apart while no move is under way, as an agent is
/// told with each result: the reason a service recorded for its failed
/// move, or a database left in the old directory that was never merged.
///
/// # Errors
/// Returns [`StateMoveError::MoveFailed`] or [`StateMoveError::LeftBehind`].
pub fn standing(location: &StateLocation) -> Result<(), StateMoveError> {
    if let Some(from) = location.pending_move()
        && let Some(reason) = recorded_move_failure(from)
    {
        return Err(StateMoveError::MoveFailed {
            from: from.to_path_buf(),
            reason,
        });
    }
    match location.left_behind() {
        Some(found) if found.join(DATABASE_FILE).is_file() => Err(StateMoveError::LeftBehind {
            found,
            to: location.in_use(),
        }),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs::{self, Permissions},
        os::unix::fs::PermissionsExt,
    };

    use super::*;
    use crate::relocation::legacy_home::{DATABASE_BYTES, LegacyHome};

    /// A folder mode every user may write, not sticky.
    const OPEN_DIR_MODE: u32 = 0o777;
    /// A folder mode of its owner's alone.
    const PRIVATE_DIR_MODE: u32 = 0o700;

    /// A due move is made, after which nothing stands in the way.
    #[test]
    fn settling_makes_a_due_move() {
        let legacy = LegacyHome::new();

        settle(&legacy.location()).unwrap();

        assert_eq!(
            fs::read(legacy.to().join(DATABASE_FILE)).unwrap(),
            DATABASE_BYTES
        );
        standing(&legacy.location()).unwrap();
    }

    /// A database left in the old directory beside an existing state
    /// directory is reported, naming the folder it really is in and the
    /// one in use, and never suggesting it be removed.
    #[test]
    fn a_database_left_in_the_legacy_dir_is_reported() {
        let legacy = LegacyHome::new();
        fs::create_dir_all(legacy.to()).unwrap();

        let error = settle(&legacy.location()).unwrap_err();

        assert!(
            matches!(error, StateMoveError::LeftBehind { .. }),
            "{error}"
        );
        let found = legacy.from().canonicalize().unwrap();
        for folder in [found, legacy.to()] {
            assert!(
                error.to_string().contains(&*folder.to_string_lossy()),
                "{error}"
            );
        }
        assert!(!error.to_string().contains("remove"), "{error}");
        fs::remove_file(legacy.from().join(DATABASE_FILE)).unwrap();
        standing(&legacy.location()).unwrap();
    }

    /// A service whose own move fails records why in the old directory, and
    /// from then on that reason is what stands in the way, for every
    /// command and agent, until the move succeeds.
    #[test]
    fn a_services_failed_move_stands_until_it_moves() {
        let legacy = LegacyHome::new();
        let parent = legacy.to().parent().unwrap().to_path_buf();
        fs::create_dir_all(&parent).unwrap();
        fs::set_permissions(&parent, Permissions::from_mode(OPEN_DIR_MODE)).unwrap();

        let failed = settle_as_service(&legacy.location()).unwrap_err();

        assert!(matches!(failed, StateMoveError::Target { .. }), "{failed}");
        let error = standing(&legacy.location()).unwrap_err();
        assert!(
            matches!(error, StateMoveError::MoveFailed { .. }),
            "{error}"
        );
        assert!(error.to_string().contains(&failed.to_string()), "{error}");

        fs::set_permissions(&parent, Permissions::from_mode(PRIVATE_DIR_MODE)).unwrap();
        settle_as_service(&legacy.location()).unwrap();
        standing(&legacy.location()).unwrap();
    }
}
