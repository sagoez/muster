use std::{
    ffi::OsString,
    fs, io,
    os::unix::fs::symlink,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

use getset::Getters;
use muster_os::{
    private_dir::{claimed_by_effective_user, create_private_dir_all},
    state_location::{StateLocation, is_absent},
};
use muster_service::Singleton;
use muster_store::DATABASE_FILE;
use typed_builder::TypedBuilder;

use super::state_move_error::StateMoveError;

/// How long a mover waits for the old directory's lock, which another mover
/// holds only for a few renames, before taking a holder to be a service.
const LOCK_WAIT: Duration = Duration::from_millis(250);
/// Pause between attempts to take that lock.
const LOCK_POLL: Duration = Duration::from_millis(10);
/// Leads the name of the link a move makes beside the old directory before
/// renaming it, then renames into its place.
const STAGED_LINK_PREFIX: &str = ".";
/// Ends that name.
const STAGED_LINK_SUFFIX: &str = ".moving";

/// The one-time move of the state directory an earlier muster kept elsewhere
/// on this platform (`~/Library/Application Support/muster` on macOS) to
/// where muster keeps it now.
///
/// A holder outlives every service and keeps the absolute paths it was
/// launched with, and its record names them, so the old place must keep
/// leading to the files: the directory is renamed and a link to it takes
/// its old place. A rename keeps every file, socket and open descriptor
/// working, and through the link a holder still rotates its log, removes
/// its socket and record on exit, and is adopted from its record. The link
/// is made first under a staged name beside the old directory, so a move
/// cut short between the rename and the link is finished by the next one.
#[derive(Debug, Clone, Getters, TypedBuilder)]
pub struct LegacyStateMove {
    /// The directory an earlier muster kept its state in.
    #[getset(get = "pub")]
    from: PathBuf,
    /// The state directory it moves to.
    #[getset(get = "pub")]
    to: PathBuf,
    /// How a directory or link is renamed: the file system's rename, or in
    /// a test one that fails where the test says.
    #[builder(default = (|from: &Path, to: &Path| fs::rename(from, to)) as Rename)]
    rename: Rename,
}

/// Renames the file at the first path to the second.
type Rename = fn(&Path, &Path) -> io::Result<()>;

impl LegacyStateMove {
    /// The move `location` still waits for, if any: the old directory to
    /// move, or the link a move cut short did not leave yet. The old
    /// directory moves only when this process's user owns it, so one run as
    /// root in a shell that kept the user's home leaves the user's state
    /// alone.
    pub fn pending(location: &StateLocation) -> Option<Self> {
        let legacy = location.legacy().as_deref()?;
        let state_move = Self::builder()
            .from(legacy.to_path_buf())
            .to(location.dir().clone())
            .build();
        let movable = location
            .pending_move()
            .is_some_and(claimed_by_effective_user);
        (movable || state_move.is_unlinked()).then_some(state_move)
    }

    /// Whether `location` left a database behind in the old directory, one
    /// that will never move because the state directory already exists or
    /// the old place leads elsewhere.
    ///
    /// # Errors
    /// Returns [`StateMoveError::LeftBehind`] naming both folders when it
    /// did.
    pub fn check_left_behind(location: &StateLocation) -> Result<(), StateMoveError> {
        match location.left_behind() {
            Some(legacy) if legacy.join(DATABASE_FILE).is_file() => {
                Err(StateMoveError::LeftBehind {
                    from: legacy.to_path_buf(),
                    to: location.dir().clone(),
                })
            },
            _ => Ok(()),
        }
    }

    /// Moves the directory, or finishes a move cut short. The move holds the
    /// old directory's service lock, so no service runs from it meanwhile,
    /// and checks once it holds it that the move is still due, so a second
    /// mover leaves the first one's work alone. A lock another mover holds
    /// is waited out briefly; one still held after that is a service's.
    ///
    /// # Errors
    /// Returns a [`StateMoveError`] saying what kept the directory from
    /// moving.
    pub fn run(&self) -> Result<(), StateMoveError> {
        if self.is_unlinked() {
            return self.finish_link();
        }
        let Some(_service_lock) = self.lock()? else {
            if !self.is_due() {
                return Ok(());
            }
            return Err(StateMoveError::ServiceRunning {
                from: self.from.clone(),
                to: self.to.clone(),
            });
        };
        if !self.is_due() {
            return Ok(());
        }
        if let Some(parent) = self.to.parent() {
            create_private_dir_all(parent).map_err(|cause| self.move_failed(cause))?;
        }
        let staged = self.staged_link();
        if fs::symlink_metadata(&staged).is_ok() {
            fs::remove_file(&staged).map_err(|cause| self.link_failed(cause))?;
        }
        symlink(&self.to, &staged).map_err(|cause| self.link_failed(cause))?;
        if let Err(cause) = (self.rename)(&self.from, &self.to) {
            let _ = fs::remove_file(&staged);
            return Err(self.move_failed(cause));
        }
        let Err(cause) = (self.rename)(&staged, &self.from) else {
            return Ok(());
        };
        if (self.rename)(&self.to, &self.from).is_err() {
            return Err(StateMoveError::Unlinked {
                from: self.from.clone(),
                to: self.to.clone(),
                cause,
            });
        }
        let _ = fs::remove_file(&staged);
        Err(self.link_failed(cause))
    }

    /// Takes the old directory's service lock, waiting up to [`LOCK_WAIT`]
    /// while it is held and the move is still due. None while it is still
    /// held, or when the old directory went meanwhile.
    ///
    /// # Errors
    /// Returns [`StateMoveError::Lock`] when the lock cannot be attempted.
    fn lock(&self) -> Result<Option<Singleton>, StateMoveError> {
        let deadline = Instant::now() + LOCK_WAIT;
        loop {
            match Singleton::acquire_existing(&self.from) {
                Ok(Some(lock)) => return Ok(Some(lock)),
                Ok(None) if Instant::now() < deadline && self.is_due() => {
                    thread::sleep(LOCK_POLL);
                },
                Ok(None) => return Ok(None),
                Err(cause) if cause.kind() == io::ErrorKind::NotFound => return Ok(None),
                Err(cause) => {
                    return Err(StateMoveError::Lock {
                        from: self.from.clone(),
                        cause,
                    });
                },
            }
        }
    }

    /// Whether the old directory still waits to move: it is a real
    /// directory and nothing is at the new place.
    fn is_due(&self) -> bool {
        let real_dir = fs::symlink_metadata(&self.from).is_ok_and(|metadata| metadata.is_dir());
        real_dir && is_absent(&self.to)
    }

    /// Whether a move was cut short after the rename: the old place is
    /// empty, the staged link waits beside it and the state directory
    /// exists.
    fn is_unlinked(&self) -> bool {
        is_absent(&self.from)
            && fs::symlink_metadata(self.staged_link()).is_ok_and(|metadata| metadata.is_symlink())
            && self.to.is_dir()
    }

    /// Puts the staged link in the old place. Another mover that put it
    /// there first counts as done.
    ///
    /// # Errors
    /// Returns [`StateMoveError::Unlinked`] when the link cannot be put.
    fn finish_link(&self) -> Result<(), StateMoveError> {
        match (self.rename)(&self.staged_link(), &self.from) {
            Err(_) if fs::read_link(&self.from).is_ok_and(|target| target == self.to) => Ok(()),
            Err(cause) => Err(StateMoveError::Unlinked {
                from: self.from.clone(),
                to: self.to.clone(),
                cause,
            }),
            Ok(()) => Ok(()),
        }
    }

    /// Where the link to the new place is made before it takes the old
    /// directory's place.
    pub(crate) fn staged_link(&self) -> PathBuf {
        let mut name = OsString::from(STAGED_LINK_PREFIX);
        if let Some(own) = self.from.file_name() {
            name.push(own);
        }
        name.push(STAGED_LINK_SUFFIX);
        self.from.with_file_name(name)
    }

    /// The failure to move this directory for `cause`, a rename across
    /// disks told apart.
    fn move_failed(&self, cause: io::Error) -> StateMoveError {
        let (from, to) = (self.from.clone(), self.to.clone());
        if cause.kind() == io::ErrorKind::CrossesDevices {
            return StateMoveError::OtherDisk { from, to };
        }
        StateMoveError::Move { from, to, cause }
    }

    /// The failure to leave the link for `cause`.
    fn link_failed(&self, cause: io::Error) -> StateMoveError {
        StateMoveError::Link {
            from: self.from.clone(),
            to: self.to.clone(),
            cause,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs::Permissions,
        os::unix::{fs::PermissionsExt, net::UnixStream},
    };

    use rustix::process::geteuid;

    use super::*;
    use crate::relocation::legacy_home::{DATABASE_BYTES, LegacyHome, STATE_DIR};

    /// A folder mode that lets its owner in but not write.
    const READ_ONLY_DIR_MODE: u32 = 0o500;
    /// A folder mode of its owner's alone.
    const PRIVATE_DIR_MODE: u32 = 0o700;

    /// The old directory moves whole and a link is left where it was, after
    /// which nothing is pending and the records it held, naming the old
    /// place, are found and still lead to the holder's files.
    #[test]
    fn the_legacy_dir_moves_and_a_link_takes_its_place() {
        let legacy = LegacyHome::new();

        legacy.state_move().run().unwrap();

        let to = legacy.to();
        assert_eq!(fs::read(to.join(DATABASE_FILE)).unwrap(), DATABASE_BYTES);
        assert_eq!(fs::read_link(legacy.from()).unwrap(), to);
        assert_eq!(LegacyHome::records_in(&to), vec![legacy.record().clone()]);
        assert!(legacy.record().log().is_file(), "the log is not reached");
        assert!(LegacyStateMove::pending(&legacy.location()).is_none());
        assert_eq!(legacy.location().in_use(), to);
        assert!(fs::symlink_metadata(legacy.staged_link()).is_err());
    }

    /// A holder that kept running across the move is reached at its socket
    /// under the new place, and through the link at the path its record
    /// names.
    #[test]
    fn a_running_holder_is_reached_at_its_new_path_and_through_the_link() {
        let legacy = LegacyHome::new();
        let launched = legacy.record().socket().clone();
        let moved = legacy
            .to()
            .join(launched.strip_prefix(legacy.from()).unwrap());

        legacy.state_move().run().unwrap();

        UnixStream::connect(&moved).unwrap();
        assert!(legacy.reached(), "the moved socket was not reached");
        UnixStream::connect(&launched).unwrap();
        assert!(legacy.reached(), "the link did not lead to the socket");
    }

    /// While a service holds the old directory nothing moves: it stays in
    /// use, and the new place is not made.
    #[test]
    fn nothing_moves_while_a_service_runs_from_the_legacy_dir() {
        let legacy = LegacyHome::new();
        let service = Singleton::acquire_existing(&legacy.from()).unwrap();
        assert!(service.is_some());

        let error = legacy.state_move().run().unwrap_err();

        assert!(
            matches!(error, StateMoveError::ServiceRunning { .. }),
            "{error}"
        );
        assert!(error.to_string().contains("muster service stop"), "{error}");
        assert!(!legacy.to().exists());
        assert_eq!(legacy.location().in_use(), legacy.from());
    }

    /// A second mover that found the move due before the first one made it
    /// leaves the moved directory and its link alone and reports nothing.
    #[test]
    fn a_second_mover_leaves_the_first_ones_move_alone() {
        let legacy = LegacyHome::new();
        let (first, second) = (legacy.state_move(), legacy.state_move());

        first.run().unwrap();
        second.run().unwrap();

        assert_eq!(fs::read_link(legacy.from()).unwrap(), legacy.to());
        assert_eq!(
            fs::read(legacy.to().join(DATABASE_FILE)).unwrap(),
            DATABASE_BYTES
        );
    }

    /// A rename that fails, here into a folder this user may not write,
    /// leaves the old directory whole and in use, with no staged link left
    /// over and nothing made at the new place.
    #[test]
    fn a_failed_rename_leaves_both_dirs_untouched() {
        if geteuid().is_root() {
            return;
        }
        let legacy = LegacyHome::new();
        let parent = legacy.to().parent().unwrap().to_path_buf();
        fs::create_dir_all(&parent).unwrap();
        fs::set_permissions(&parent, Permissions::from_mode(READ_ONLY_DIR_MODE)).unwrap();

        let error = legacy.state_move().run().unwrap_err();

        fs::set_permissions(&parent, Permissions::from_mode(PRIVATE_DIR_MODE)).unwrap();
        assert!(matches!(error, StateMoveError::Move { .. }), "{error}");
        assert_eq!(
            fs::read(legacy.from().join(DATABASE_FILE)).unwrap(),
            DATABASE_BYTES
        );
        assert!(!legacy.to().exists());
        assert!(fs::symlink_metadata(legacy.staged_link()).is_err());
        assert_eq!(legacy.location().in_use(), legacy.from());
    }

    /// Once the state directory exists nothing is moved, and a database
    /// still in the old one is reported as left behind, naming both.
    #[test]
    fn a_database_left_in_the_legacy_dir_is_reported() {
        let legacy = LegacyHome::new();
        fs::create_dir_all(legacy.to()).unwrap();

        assert!(LegacyStateMove::pending(&legacy.location()).is_none());
        let error = LegacyStateMove::check_left_behind(&legacy.location()).unwrap_err();

        assert!(
            matches!(error, StateMoveError::LeftBehind { .. }),
            "{error}"
        );
        for folder in [legacy.from(), legacy.to()] {
            assert!(
                error.to_string().contains(&*folder.to_string_lossy()),
                "{error}"
            );
        }
        fs::remove_file(legacy.from().join(DATABASE_FILE)).unwrap();
        LegacyStateMove::check_left_behind(&legacy.location()).unwrap();
    }

    /// A move cut short between the rename and the link is finished by the
    /// next one, which puts the staged link in the old place.
    #[test]
    fn a_move_cut_short_before_its_link_is_finished_by_the_next() {
        let legacy = LegacyHome::new();
        fs::create_dir_all(legacy.to().parent().unwrap()).unwrap();
        symlink(legacy.to(), legacy.staged_link()).unwrap();
        fs::rename(legacy.from(), legacy.to()).unwrap();

        let finishing = LegacyStateMove::pending(&legacy.location()).unwrap();
        finishing.run().unwrap();

        assert_eq!(fs::read_link(legacy.from()).unwrap(), legacy.to());
        assert!(fs::symlink_metadata(legacy.staged_link()).is_err());
        assert!(LegacyStateMove::pending(&legacy.location()).is_none());
    }

    /// Renames as the file system does, except a link, which cannot be put.
    fn refusing_links(from: &Path, to: &Path) -> io::Result<()> {
        if is_link(from) {
            return Err(io::Error::from(io::ErrorKind::PermissionDenied));
        }
        fs::rename(from, to)
    }

    /// Renames as [`refusing_links`] does, and cannot move the new state
    /// directory back either.
    fn refusing_links_and_the_way_back(from: &Path, to: &Path) -> io::Result<()> {
        if from.ends_with(STATE_DIR) {
            return Err(io::Error::from(io::ErrorKind::PermissionDenied));
        }
        refusing_links(from, to)
    }

    /// Renames nothing, as across disks.
    fn across_disks(_from: &Path, _to: &Path) -> io::Result<()> {
        Err(io::Error::from(io::ErrorKind::CrossesDevices))
    }

    /// Whether `path` is a link.
    fn is_link(path: &Path) -> bool {
        fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_symlink())
    }

    /// A link that cannot be put in the old place after the rename moves
    /// the directory back, so it stays whole and in use, with no staged
    /// link left over.
    #[test]
    fn a_failed_final_link_moves_the_dir_back() {
        let legacy = LegacyHome::new();

        let error = legacy
            .state_move_renaming(refusing_links)
            .run()
            .unwrap_err();

        assert!(matches!(error, StateMoveError::Link { .. }), "{error}");
        assert_eq!(
            fs::read(legacy.from().join(DATABASE_FILE)).unwrap(),
            DATABASE_BYTES
        );
        assert!(!legacy.to().exists());
        assert!(fs::symlink_metadata(legacy.staged_link()).is_err());
        assert_eq!(legacy.location().in_use(), legacy.from());
    }

    /// A moved directory that can neither take its link nor move back keeps
    /// the staged link, and the next move puts it in place.
    #[test]
    fn a_dir_that_cannot_move_back_is_linked_by_the_next_move() {
        let legacy = LegacyHome::new();

        let error = legacy
            .state_move_renaming(refusing_links_and_the_way_back)
            .run()
            .unwrap_err();

        assert!(matches!(error, StateMoveError::Unlinked { .. }), "{error}");
        assert!(is_link(&legacy.staged_link()), "the staged link was lost");
        LegacyStateMove::pending(&legacy.location())
            .unwrap()
            .run()
            .unwrap();
        assert_eq!(fs::read_link(legacy.from()).unwrap(), legacy.to());
    }

    /// A move across disks says so and what to do, and leaves the old
    /// directory whole and in use, with no staged link left over.
    #[test]
    fn a_move_across_disks_says_so() {
        let legacy = LegacyHome::new();

        let error = legacy.state_move_renaming(across_disks).run().unwrap_err();

        assert!(matches!(error, StateMoveError::OtherDisk { .. }), "{error}");
        assert!(error.to_string().contains("another disk"), "{error}");
        assert!(error.to_string().contains("XDG_STATE_HOME"), "{error}");
        assert_eq!(
            fs::read(legacy.from().join(DATABASE_FILE)).unwrap(),
            DATABASE_BYTES
        );
        assert!(fs::symlink_metadata(legacy.staged_link()).is_err());
        assert_eq!(legacy.location().in_use(), legacy.from());
    }
}
