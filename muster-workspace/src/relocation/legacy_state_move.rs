use std::{
    ffi::OsString,
    fs, io,
    os::unix::fs::{MetadataExt, symlink},
    path::{Path, PathBuf},
    process, thread,
    time::{Duration, Instant},
};

use getset::Getters;
use muster_os::{
    path::{is_absent, is_link, is_real_dir},
    private_dir::{check_move_folder, create_private_dir_all, refuse_making},
    state_location::{STATE_HOME_ENV, StateLocation},
};
use muster_service::Singleton;
use rustix::process::geteuid;
use typed_builder::TypedBuilder;

use super::{
    move_failure::{clear_move_failure, recorded_move_failure},
    state_move_error::StateMoveError,
};

/// How long a mover waits for the old directory's lock, which another mover
/// holds only for a few renames, before taking a holder to be a service.
const LOCK_WAIT: Duration = Duration::from_millis(50);
/// Pause between attempts to take that lock.
const LOCK_POLL: Duration = Duration::from_millis(10);
/// Leads the name of the link a move makes beside the old directory before
/// renaming it, then renames into its place.
const STAGED_LINK_PREFIX: &str = ".";
/// Ends that name.
const STAGED_LINK_SUFFIX: &str = ".moving";
/// Ends the name a stray directory made in the old place is set aside under,
/// before this process's id.
const STRAY_SUFFIX: &str = ".stray-";

/// Renames the file at the first path to the second.
pub(crate) type Rename = fn(&Path, &Path) -> io::Result<()>;

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

impl LegacyStateMove {
    /// The move `location` still waits for, if any: the old directory to
    /// move, or the link a move cut short did not leave yet. The old
    /// directory moves only when this process's user owns it and the folder
    /// it sits in, so one run as root in a shell that kept a user's home
    /// never moves that user's state.
    pub fn pending(location: &StateLocation) -> Option<Self> {
        let legacy = location.legacy().as_deref()?;
        let state_move = Self::builder()
            .from(legacy.to_path_buf())
            .to(location.dir().clone())
            .build();
        let movable = location
            .pending_move()
            .is_some_and(|from| movable_by(from, geteuid().as_raw()));
        let unfinished = state_move.is_unlinked() || state_move.has_stray();
        (movable || unfinished).then_some(state_move)
    }

    /// Moves the directory, or finishes a move cut short. The move holds the
    /// old directory's service lock, so no service runs from it meanwhile,
    /// and checks once it holds it that the move is still due, so a second
    /// mover leaves the first one's work alone. A lock held by a service of
    /// this version, which records why its own move failed, is reported with
    /// that reason at once; one held by an older service is waited out
    /// briefly, as another mover holds it only for a moment, then reported.
    /// The new place goes only where this user may make it, in a folder of
    /// theirs no one else may write.
    ///
    /// # Errors
    /// Returns a [`StateMoveError`] saying what kept the directory from
    /// moving.
    pub fn run(&self) -> Result<(), StateMoveError> {
        if self.has_stray() {
            self.set_stray_aside()?;
        }
        if self.is_unlinked() {
            return self.finish_link();
        }
        let Some(_service_lock) = self.lock()? else {
            return self.blocked();
        };
        if !self.is_due() {
            return Ok(());
        }
        let user = geteuid().as_raw();
        refuse_making(&self.to, user, None).map_err(|cause| self.target_refused(cause))?;
        if let Some(parent) = self.to.parent() {
            create_private_dir_all(parent).map_err(|cause| self.move_failed(cause))?;
        }
        check_move_folder(&self.to, user).map_err(|cause| self.target_refused(cause))?;
        let staged = self.staged_link();
        if fs::symlink_metadata(&staged).is_ok() {
            fs::remove_file(&staged).map_err(|cause| self.link_failed(cause))?;
        }
        symlink(&self.to, &staged).map_err(|cause| self.link_failed(cause))?;
        if let Err(cause) = (self.rename)(&self.from, &self.to) {
            let _ = fs::remove_file(&staged);
            return Err(self.move_failed(cause));
        }
        let cause = match (self.rename)(&staged, &self.from) {
            Err(cause) if !self.links_here() => cause,
            _ => {
                clear_move_failure(&self.to);
                return Ok(());
            },
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

    /// Why the move did not happen while the lock is held by someone else:
    /// nothing when it no longer waits, the reason a service of this
    /// version recorded for its own failed move, else an older service.
    ///
    /// # Errors
    /// Returns [`StateMoveError::MoveFailed`] or
    /// [`StateMoveError::ServiceRunning`].
    fn blocked(&self) -> Result<(), StateMoveError> {
        if !self.is_due() {
            return Ok(());
        }
        if let Some(reason) = recorded_move_failure(&self.from) {
            return Err(StateMoveError::MoveFailed {
                from: self.from.clone(),
                reason,
            });
        }
        Err(StateMoveError::ServiceRunning {
            from: self.from.clone(),
            to: self.to.clone(),
        })
    }

    /// Takes the old directory's service lock. While it is held and the
    /// move is still due it is tried again for up to [`LOCK_WAIT`], unless
    /// a service of this version, which recorded why its move failed, is
    /// what holds it. None while it is still held, or when the old
    /// directory went meanwhile.
    ///
    /// # Errors
    /// Returns [`StateMoveError::Lock`] when the lock cannot be attempted.
    fn lock(&self) -> Result<Option<Singleton>, StateMoveError> {
        let deadline = Instant::now() + LOCK_WAIT;
        loop {
            match Singleton::acquire_existing(&self.from) {
                Ok(Some(lock)) => return Ok(Some(lock)),
                Ok(None)
                    if Instant::now() < deadline
                        && self.is_due()
                        && recorded_move_failure(&self.from).is_none() =>
                {
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
        is_real_dir(&self.from) && is_absent(&self.to)
    }

    /// Whether the staged link waits beside the old place and leads to the
    /// state directory, which exists.
    fn is_staged(&self) -> bool {
        let staged = self.staged_link();
        is_link(&staged)
            && fs::read_link(&staged).is_ok_and(|target| target == self.to)
            && self.to.is_dir()
    }

    /// Whether a move was cut short after the rename: the old place is
    /// empty and the staged link waits beside it.
    fn is_unlinked(&self) -> bool {
        is_absent(&self.from) && self.is_staged()
    }

    /// Whether a move cut short after the rename found a real directory made
    /// in the old place meanwhile, as something that resolved the old place
    /// before the move may make: the staged link still waits beside it.
    fn has_stray(&self) -> bool {
        is_real_dir(&self.from) && self.is_staged()
    }

    /// Whether the old place is the link to the new one, as when another
    /// mover put the staged link there first.
    fn links_here(&self) -> bool {
        fs::read_link(&self.from).is_ok_and(|target| target == self.to)
    }

    /// Sets the directory made in the old place after the move aside, under
    /// a name of its own beside it, so the link can take its place.
    ///
    /// # Errors
    /// Returns [`StateMoveError::Unlinked`] when it cannot be set aside.
    fn set_stray_aside(&self) -> Result<(), StateMoveError> {
        let mut name = self.from.as_os_str().to_owned();
        name.push(STRAY_SUFFIX);
        name.push(process::id().to_string());
        match (self.rename)(&self.from, Path::new(&name)) {
            Err(_) if !is_real_dir(&self.from) => Ok(()),
            Err(cause) => Err(StateMoveError::Unlinked {
                from: self.from.clone(),
                to: self.to.clone(),
                cause,
            }),
            Ok(()) => Ok(()),
        }
    }

    /// Puts the staged link in the old place. Another mover that put it
    /// there first counts as done.
    ///
    /// # Errors
    /// Returns [`StateMoveError::Unlinked`] when the link cannot be put.
    fn finish_link(&self) -> Result<(), StateMoveError> {
        match (self.rename)(&self.staged_link(), &self.from) {
            Err(_) if self.links_here() => Ok(()),
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
            return StateMoveError::OtherDisk {
                from,
                to,
                variable: STATE_HOME_ENV,
            };
        }
        StateMoveError::Move { from, to, cause }
    }

    /// The refusal to move into a place this user may not keep state in.
    fn target_refused(&self, cause: io::Error) -> StateMoveError {
        StateMoveError::Target {
            from: self.from.clone(),
            cause,
        }
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

/// Whether the user `uid` may move `legacy`: they own it and the folder it
/// sits in, so the folder is theirs, not another user's home seen as root.
fn movable_by(legacy: &Path, uid: u32) -> bool {
    let owned =
        |path: &Path| fs::symlink_metadata(path).is_ok_and(|metadata| metadata.uid() == uid);
    owned(legacy) && legacy.parent().is_some_and(owned)
}

#[cfg(test)]
mod tests {
    use std::{
        fs::Permissions,
        os::unix::{fs::PermissionsExt, net::UnixStream},
    };

    use muster_store::DATABASE_FILE;

    use super::*;
    use crate::relocation::{
        legacy_home::{DATABASE_BYTES, LegacyHome, STATE_DIR},
        move_failure::record_move_failure,
    };

    /// A folder mode that lets its owner in but not write.
    const READ_ONLY_DIR_MODE: u32 = 0o500;
    /// A folder mode of its owner's alone.
    const PRIVATE_DIR_MODE: u32 = 0o700;
    /// A folder mode every user may write, not sticky.
    const OPEN_DIR_MODE: u32 = 0o777;
    /// The reason a test records for a service's failed move.
    const RECORDED_REASON: &str = "the disk is full";
    /// A file a stray directory in the old place holds.
    const STRAY_FILE: &str = "stray";
    /// A folder no test makes, under the root folder.
    const ABSENT_ROOT_FOLDER: &str = "/muster-absent-state";

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

    /// While an older service holds the old directory nothing moves: it
    /// stays in use, the new place is not made, and the report says how
    /// the handover or a stop moves it.
    #[test]
    fn nothing_moves_while_an_older_service_runs_from_the_legacy_dir() {
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

    /// A service of this version that holds the old directory because its
    /// own move failed is reported with the reason it recorded, at once,
    /// with no advice to stop it.
    #[test]
    fn a_service_whose_move_failed_is_reported_with_its_reason() {
        let legacy = LegacyHome::new();
        let service = Singleton::acquire_existing(&legacy.from()).unwrap();
        assert!(service.is_some());
        record_move_failure(&legacy.from(), RECORDED_REASON);

        let started = Instant::now();
        let error = legacy.state_move().run().unwrap_err();

        assert!(started.elapsed() < LOCK_WAIT, "the lock was waited out");
        assert!(
            matches!(error, StateMoveError::MoveFailed { .. }),
            "{error}"
        );
        assert!(error.to_string().contains(RECORDED_REASON), "{error}");
        assert!(!error.to_string().contains("service stop"), "{error}");
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

    /// The state never moves into a folder other users may write, which
    /// they could swap it out of, and nothing moves.
    #[test]
    fn nothing_moves_into_a_folder_others_may_write() {
        let legacy = LegacyHome::new();
        let parent = legacy.to().parent().unwrap().to_path_buf();
        fs::create_dir_all(&parent).unwrap();
        fs::set_permissions(&parent, Permissions::from_mode(OPEN_DIR_MODE)).unwrap();

        let error = legacy.state_move().run().unwrap_err();

        assert!(matches!(error, StateMoveError::Target { .. }), "{error}");
        assert!(legacy.from().join(DATABASE_FILE).is_file());
        assert!(!legacy.to().exists());
    }

    /// Nothing is made, and nothing moves, under a folder another user
    /// owns, as root would under a user's home.
    #[test]
    fn nothing_moves_under_another_users_folder() {
        if geteuid().is_root() {
            return;
        }
        let legacy = LegacyHome::new();
        let to = Path::new(ABSENT_ROOT_FOLDER).join(STATE_DIR);

        let error = legacy.state_move_to(&to).run().unwrap_err();

        assert!(matches!(error, StateMoveError::Target { .. }), "{error}");
        assert!(!Path::new(ABSENT_ROOT_FOLDER).exists());
        assert!(legacy.from().join(DATABASE_FILE).is_file());
    }

    /// Only a user who owns the old directory and its folder moves it, so
    /// root in a shell that kept a user's home never does.
    #[test]
    fn only_the_owner_of_the_legacy_dir_and_its_folder_moves_it() {
        let legacy = LegacyHome::new();
        let user = geteuid().as_raw();

        assert!(movable_by(&legacy.from(), user));
        assert!(!movable_by(&legacy.from(), user + 1));
    }

    /// Once the state directory exists nothing is moved.
    #[test]
    fn nothing_moves_once_the_state_dir_exists() {
        let legacy = LegacyHome::new();
        fs::create_dir_all(legacy.to()).unwrap();

        assert!(LegacyStateMove::pending(&legacy.location()).is_none());
        assert!(legacy.from().join(DATABASE_FILE).is_file());
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

    /// A staged link that leads elsewhere than the state directory is never
    /// put in the old place.
    #[test]
    fn a_staged_link_leading_elsewhere_is_not_put_in_place() {
        let legacy = LegacyHome::new();
        fs::create_dir_all(legacy.to().parent().unwrap()).unwrap();
        let elsewhere = legacy.to().with_file_name(STRAY_FILE);
        fs::create_dir(&elsewhere).unwrap();
        symlink(&elsewhere, legacy.staged_link()).unwrap();
        fs::rename(legacy.from(), legacy.to()).unwrap();

        assert!(LegacyStateMove::pending(&legacy.location()).is_none());
        assert!(is_absent(&legacy.from()));
    }

    /// A directory made in the old place after a move cut short, as by
    /// something that resolved the old place before the move, is set aside
    /// and the link put in its place.
    #[test]
    fn a_stray_dir_in_the_old_place_is_set_aside_for_the_link() {
        let legacy = LegacyHome::new();
        fs::create_dir_all(legacy.to().parent().unwrap()).unwrap();
        symlink(legacy.to(), legacy.staged_link()).unwrap();
        fs::rename(legacy.from(), legacy.to()).unwrap();
        fs::create_dir(legacy.from()).unwrap();
        fs::write(legacy.from().join(STRAY_FILE), b"").unwrap();

        LegacyStateMove::pending(&legacy.location())
            .unwrap()
            .run()
            .unwrap();

        assert_eq!(fs::read_link(legacy.from()).unwrap(), legacy.to());
        assert!(
            fs::read_dir(legacy.from().parent().unwrap())
                .unwrap()
                .filter_map(Result::ok)
                .any(|entry| entry.file_name().to_string_lossy().contains(STRAY_SUFFIX))
        );
    }

    /// A successful move clears a failure a service recorded before.
    #[test]
    fn a_successful_move_clears_a_recorded_failure() {
        let legacy = LegacyHome::new();
        record_move_failure(&legacy.from(), RECORDED_REASON);

        legacy.state_move().run().unwrap();

        assert_eq!(recorded_move_failure(&legacy.to()), None);
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

    /// Renames as the file system does, but a link loses the race to put it
    /// to another mover, which puts it first.
    fn losing_the_link_to_another_mover(from: &Path, to: &Path) -> io::Result<()> {
        fs::rename(from, to)?;
        if is_link(to) {
            return Err(io::Error::from(io::ErrorKind::NotFound));
        }
        Ok(())
    }

    /// Renames nothing, as across disks.
    fn across_disks(_from: &Path, _to: &Path) -> io::Result<()> {
        Err(io::Error::from(io::ErrorKind::CrossesDevices))
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

    /// A mover whose link another mover put first reports the move done.
    #[test]
    fn a_link_another_mover_put_first_counts_as_done() {
        let legacy = LegacyHome::new();

        legacy
            .state_move_renaming(losing_the_link_to_another_mover)
            .run()
            .unwrap();

        assert_eq!(fs::read_link(legacy.from()).unwrap(), legacy.to());
    }

    /// A move across disks says so and names the variable that moves the
    /// state, and leaves the old directory whole and in use, with no staged
    /// link left over.
    #[test]
    fn a_move_across_disks_says_so() {
        let legacy = LegacyHome::new();

        let error = legacy.state_move_renaming(across_disks).run().unwrap_err();

        assert!(matches!(error, StateMoveError::OtherDisk { .. }), "{error}");
        assert!(error.to_string().contains("another disk"), "{error}");
        assert!(error.to_string().contains(STATE_HOME_ENV), "{error}");
        assert_eq!(
            fs::read(legacy.from().join(DATABASE_FILE)).unwrap(),
            DATABASE_BYTES
        );
        assert!(fs::symlink_metadata(legacy.staged_link()).is_err());
        assert_eq!(legacy.location().in_use(), legacy.from());
    }
}
