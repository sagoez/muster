use std::{
    fs, io,
    os::unix::fs::symlink,
    path::{Path, PathBuf},
};

use getset::Getters;
use muster_os::{
    private_dir::{claimed_by_effective_user, create_private_dir_all},
    state_location::StateLocation,
};
use muster_protocol::holder::{HolderDirs, HolderError, HolderRecord};
use muster_service::Singleton;
use typed_builder::TypedBuilder;

use super::state_move_error::StateMoveError;

/// The one-time move of the state directory an earlier muster kept elsewhere
/// on this platform (`~/Library/Application Support/muster` on macOS) to
/// where muster keeps it now.
///
/// A holder outlives every service and keeps the absolute paths it was
/// launched with, so the move keeps running processes adoptable: it happens
/// only while no service runs from the old directory and none can start
/// there, the holder records are pointed at the new directory before it is
/// renamed, and a link is left in its old place. A rename keeps every file,
/// socket and open descriptor working; through the link a holder still
/// rotates its log and removes its socket and record on exit.
#[derive(Debug, Clone, PartialEq, Eq, Getters, TypedBuilder)]
pub struct LegacyStateMove {
    /// The directory an earlier muster kept its state in.
    #[getset(get = "pub")]
    from: PathBuf,
    /// The state directory it moves to, which does not exist yet.
    #[getset(get = "pub")]
    to: PathBuf,
}

impl LegacyStateMove {
    /// The move `location` still waits for, if any, and only when this
    /// process's user owns the old directory: one run as root in a shell
    /// that kept the user's home leaves the user's state alone.
    pub fn pending(location: &StateLocation) -> Option<Self> {
        let pending = location.pending_move();
        pending
            .filter(|from| claimed_by_effective_user(from))
            .map(|from| {
                Self::builder()
                    .from(from.to_path_buf())
                    .to(location.dir().clone())
                    .build()
            })
    }

    /// Moves the directory, holding the service lock of the old one so no
    /// service runs from it meanwhile. Records are pointed at the new place
    /// first, so a move cut short is finished by the next one, and pointed
    /// back when the rename fails, as across file systems, leaving the old
    /// directory whole and in use.
    ///
    /// # Errors
    /// Returns a [`StateMoveError`] saying what kept the directory from
    /// moving, or, once it has, that no link could be left behind.
    pub fn run(&self) -> Result<(), StateMoveError> {
        let lock = Singleton::acquire(&self.from).map_err(|source| StateMoveError::Lock {
            from: self.from.clone(),
            source,
        })?;
        let Some(_service_lock) = lock else {
            return Err(StateMoveError::ServiceRunning {
                from: self.from.clone(),
                to: self.to.clone(),
            });
        };
        if let Some(parent) = self.to.parent() {
            create_private_dir_all(parent).map_err(|source| self.move_failed(source))?;
        }
        let records = HolderDirs::under(&self.from).records().clone();
        if let Err(source) = rebase_records(&records, &self.from, &self.to) {
            let _ = rebase_records(&records, &self.to, &self.from);
            return Err(StateMoveError::Records {
                from: self.from.clone(),
                to: self.to.clone(),
                source,
            });
        }
        if let Err(source) = fs::rename(&self.from, &self.to) {
            let _ = rebase_records(&records, &self.to, &self.from);
            return Err(self.move_failed(source));
        }
        symlink(&self.to, &self.from).map_err(|source| StateMoveError::Link {
            from: self.from.clone(),
            to: self.to.clone(),
            source,
        })
    }

    /// The failure to move this directory for `source`.
    fn move_failed(&self, source: io::Error) -> StateMoveError {
        StateMoveError::Move {
            from: self.from.clone(),
            to: self.to.clone(),
            source,
        }
    }
}

/// Rewrites each record in `dir` whose socket or log lies under `from` to
/// name the same place under `to`. Records already pointing elsewhere, or
/// unreadable, are left as they are.
///
/// # Errors
/// Returns the [`HolderError`] of the first record that cannot be saved.
fn rebase_records(dir: &Path, from: &Path, to: &Path) -> Result<(), HolderError> {
    HolderRecord::load_all(dir)
        .iter()
        .filter_map(|record| record.rebased(from, to))
        .try_for_each(|record| record.save(dir))
}

#[cfg(test)]
mod tests {
    use std::os::unix::net::UnixStream;

    use super::*;
    use crate::relocation::legacy_home::{DATABASE, DATABASE_BYTES, LegacyHome};

    /// A file the state directory already holds.
    const KEPT_FILE: &str = "kept";

    /// The old directory moves whole, its records name the new place, and a
    /// link is left where it was, after which nothing is pending.
    #[test]
    fn the_legacy_dir_moves_and_its_records_name_the_new_place() {
        let legacy = LegacyHome::new();
        let state_move = LegacyStateMove::pending(&legacy.location()).unwrap();
        assert_eq!(state_move, legacy.state_move());

        state_move.run().unwrap();

        let to = legacy.to();
        assert_eq!(fs::read(to.join(DATABASE)).unwrap(), DATABASE_BYTES);
        assert_eq!(fs::read_link(legacy.from()).unwrap(), to);
        let moved = legacy.moved_record();
        assert_eq!(LegacyHome::records_in(&to), vec![moved.clone()]);
        assert!(
            moved.socket().starts_with(&to),
            "{}",
            moved.socket().display()
        );
        assert!(moved.log().is_file(), "{}", moved.log().display());
        assert_eq!(LegacyStateMove::pending(&legacy.location()), None);
        assert_eq!(legacy.location().in_use(), to);
    }

    /// A holder that kept running across the move is reached at the socket
    /// its rewritten record names, and through the link at the path it was
    /// launched with, where it rotates its log and removes its socket.
    #[test]
    fn a_running_holder_is_reached_at_its_new_path_and_through_the_link() {
        let legacy = LegacyHome::new();

        legacy.state_move().run().unwrap();

        UnixStream::connect(legacy.moved_record().socket()).unwrap();
        assert!(legacy.reached(), "the moved socket was not reached");
        UnixStream::connect(legacy.record().socket()).unwrap();
        assert!(legacy.reached(), "the link did not lead to the socket");
        assert!(legacy.record().log().is_file(), "the log is not reached");
    }

    /// While a service holds the old directory nothing moves: it stays in
    /// use with its records untouched, and the new place is not made.
    #[test]
    fn nothing_moves_while_a_service_runs_from_the_legacy_dir() {
        let legacy = LegacyHome::new();
        let service = Singleton::acquire(&legacy.from()).unwrap();
        assert!(service.is_some());

        let error = legacy.state_move().run().unwrap_err();

        assert!(
            matches!(error, StateMoveError::ServiceRunning { .. }),
            "{error}"
        );
        assert!(error.to_string().contains("muster service stop"), "{error}");
        assert!(!legacy.to().exists());
        assert_eq!(LegacyHome::records_in(&legacy.from()), vec![
            legacy.record().clone()
        ]);
        assert_eq!(legacy.location().in_use(), legacy.from());
    }

    /// A rename that fails leaves both directories as they were, with the
    /// records pointed back at the old one, which stays whole.
    #[test]
    fn a_failed_rename_leaves_both_dirs_untouched() {
        let legacy = LegacyHome::new();
        fs::create_dir_all(legacy.to()).unwrap();
        fs::write(legacy.to().join(KEPT_FILE), b"").unwrap();

        let error = legacy.state_move().run().unwrap_err();

        assert!(matches!(error, StateMoveError::Move { .. }), "{error}");
        assert_eq!(
            fs::read(legacy.from().join(DATABASE)).unwrap(),
            DATABASE_BYTES
        );
        assert_eq!(LegacyHome::records_in(&legacy.from()), vec![
            legacy.record().clone()
        ]);
        let kept: Vec<_> = fs::read_dir(legacy.to()).unwrap().collect();
        assert_eq!(kept.len(), 1, "the state directory changed");
    }

    /// Once the state directory exists nothing is moved, though the old one
    /// is still there.
    #[test]
    fn nothing_moves_once_the_state_dir_exists() {
        let legacy = LegacyHome::new();
        fs::create_dir_all(legacy.to()).unwrap();

        assert_eq!(LegacyStateMove::pending(&legacy.location()), None);
        assert_eq!(legacy.location().in_use(), legacy.to());
        assert!(legacy.from().join(DATABASE).is_file());
    }

    /// A move cut short after its records were rewritten, before the
    /// rename, is finished by the next one.
    #[test]
    fn a_move_cut_short_is_finished_by_the_next() {
        let legacy = LegacyHome::new();
        let records = HolderDirs::under(&legacy.from()).records().clone();
        rebase_records(&records, &legacy.from(), &legacy.to()).unwrap();
        assert_eq!(legacy.location().in_use(), legacy.from());

        legacy.state_move().run().unwrap();

        assert_eq!(LegacyHome::records_in(&legacy.to()), vec![
            legacy.moved_record()
        ]);
    }
}
