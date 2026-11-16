//! Directories only their owner can enter, for everything muster keeps about
//! a user's processes: logs, records, sockets and shared state.

use std::{
    env,
    fs::{self, DirBuilder, Permissions},
    io,
    os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
};

use muster_domain::constants::APP_NAME;
use rustix::process::geteuid;

use super::{
    no_state_dir::NoStateDir,
    state_location::{StateLocation, is_absent},
};

/// Application directory name under the runtime directory.
const APP_DIR: &str = APP_NAME;

/// Mode of a private directory: its owner's alone.
pub const PRIVATE_DIR_MODE: u32 = 0o700;
/// Mode bits granting the group or other users any access.
pub const GROUP_OTHER_BITS: u32 = 0o077;
/// The permission bits of a mode.
const PERMISSION_BITS: u32 = 0o777;
/// Mode bit of a folder in which only an entry's owner may rename or remove
/// it, as `/tmp` has.
const STICKY_BIT: u32 = 0o1000;
/// Variable naming the user's runtime directory, where the service socket
/// goes; set it to a shorter folder when the socket path does not fit.
pub const RUNTIME_DIR_ENV: &str = "XDG_RUNTIME_DIR";
/// Short folder the socket directory goes in when no runtime directory is
/// set, as on macOS.
const FALLBACK_SOCKET_ROOT: &str = "/tmp";
/// Name of the fallback socket directory, before the user's uid.
const FALLBACK_SOCKET_PREFIX: &str = "muster-";

/// muster's state directory: `muster` in `$XDG_STATE_HOME` when that is
/// absolute, else `~/.local/state/muster`, on every platform. An earlier
/// muster's directory that has not moved there yet stays in use until it
/// has, as [`StateLocation::in_use`] says.
///
/// # Errors
/// Returns [`NoStateDir`] when no home folder can be found.
pub fn state_dir() -> Result<PathBuf, NoStateDir> {
    StateLocation::current().map(|location| location.in_use().to_path_buf())
}

/// The short directory the service socket goes in, so its path fits a Unix
/// socket address wherever the state directory is: `muster` in
/// `$XDG_RUNTIME_DIR`, else `/tmp/muster-<uid>`.
pub fn socket_dir() -> PathBuf {
    env::var_os(RUNTIME_DIR_ENV)
        .map(PathBuf::from)
        .filter(|runtime| runtime.is_absolute())
        .map_or_else(
            || {
                Path::new(FALLBACK_SOCKET_ROOT)
                    .join(format!("{FALLBACK_SOCKET_PREFIX}{}", geteuid().as_raw()))
            },
            |runtime| runtime.join(APP_DIR),
        )
}

/// Readies `dir` for the service socket: creates it and any missing parents
/// owner-only, then refuses it as [`check_socket_dir`] does, since it lies in
/// a folder other users may write to. The service binds there, and a client
/// connects only after this, so neither trusts a directory another user
/// made or can enter.
///
/// # Errors
/// Returns an error of kind [`io::ErrorKind::PermissionDenied`] naming the
/// directory when it is not private to this user, or the I/O error from
/// creating or inspecting it.
pub fn prepare_socket_dir(dir: &Path) -> io::Result<()> {
    create_private_dir_all(dir)?;
    check_socket_dir(dir)
}

/// Refuses `dir` unless it is a real directory, not a link, this user owns
/// with mode 0700.
///
/// # Errors
/// Returns an error of kind [`io::ErrorKind::PermissionDenied`] naming the
/// directory when it is not private to this user, or the I/O error from
/// inspecting it, of kind [`io::ErrorKind::NotFound`] when it is absent.
pub fn check_socket_dir(dir: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(dir)?;
    let private = metadata.is_dir()
        && metadata.uid() == geteuid().as_raw()
        && metadata.permissions().mode() & PERMISSION_BITS == PRIVATE_DIR_MODE;
    if private {
        return Ok(());
    }
    Err(io::Error::new(
        io::ErrorKind::PermissionDenied,
        format!(
            "the socket directory {} is not a directory owned by you with mode \
             {PRIVATE_DIR_MODE:04o}, so muster will not use a socket there; if another user \
             made it, set {RUNTIME_DIR_ENV} to a directory of your own to move the socket",
            dir.display()
        ),
    ))
}

/// Creates `dir` and any missing parents owner-only. An existing directory is
/// left as it is.
///
/// # Errors
/// Returns the I/O error from creating it.
pub fn create_private_dir_all(dir: &Path) -> io::Result<()> {
    DirBuilder::new()
        .recursive(true)
        .mode(PRIVATE_DIR_MODE)
        .create(dir)
}

/// Whether this process's effective user owns `dir`, creating it first as
/// [`claimed_by`] does, so what runs as another user, root in a `sudo -s`
/// shell that kept the user's home, leaves it alone. The old place an
/// earlier muster kept its state in is never made again once it moved.
pub fn claimed_by_effective_user(dir: &Path) -> bool {
    let moved = is_absent(dir) && legacy_state_dir().as_deref() == Some(dir);
    !moved && claimed_by(dir, geteuid().as_raw())
}

/// The old place an earlier muster kept its state in, when this platform
/// had one apart from today's.
fn legacy_state_dir() -> Option<PathBuf> {
    StateLocation::current()
        .ok()
        .and_then(|location| location.legacy().clone())
}

/// Refuses to make the absent state directory `dir` as the user `uid` when
/// it is `legacy`, the old place a move took away, which whatever resolved
/// it before the move must resolve again instead, or when the nearest
/// folder above it that exists belongs to another user and is not sticky,
/// as a user's home is to root.
///
/// # Errors
/// Returns an error of kind [`io::ErrorKind::NotFound`] for the moved place
/// and of kind [`io::ErrorKind::PermissionDenied`] for another user's
/// folder, each naming the directory.
fn refuse_making(dir: &Path, uid: u32, legacy: Option<&Path>) -> io::Result<()> {
    if !is_absent(dir) {
        return Ok(());
    }
    if legacy == Some(dir) {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!(
                "the state directory {} has moved; run the command again",
                dir.display()
            ),
        ));
    }
    let above = dir
        .ancestors()
        .skip(1)
        .find_map(|folder| fs::metadata(folder).ok().map(|metadata| (folder, metadata)));
    match above {
        Some((folder, metadata)) if metadata.uid() != uid && metadata.mode() & STICKY_BIT == 0 => {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "muster will not make the state directory {} inside {}, which belongs to \
                     another user (uid {})",
                    dir.display(),
                    folder.display(),
                    metadata.uid()
                ),
            ))
        },
        _ => Ok(()),
    }
}

/// Whether the user `uid` owns `dir`, creating it owner-only first when it
/// is absent and `uid` owns its parent. A missing parent is never made, and
/// a directory another user owns, or made meanwhile, is not claimed.
pub fn claimed_by(dir: &Path, uid: u32) -> bool {
    if fs::symlink_metadata(dir).is_err() {
        if !dir.parent().is_some_and(|parent| owned_by(parent, uid)) {
            return false;
        }
        let _ = DirBuilder::new().mode(PRIVATE_DIR_MODE).create(dir);
    }
    owned_by(dir, uid)
}

/// Whether `dir` exists and the user `uid` owns it.
fn owned_by(dir: &Path, uid: u32) -> bool {
    fs::metadata(dir).is_ok_and(|metadata| metadata.uid() == uid)
}

/// Readies muster's state directory: creates it and any missing parents
/// owner-only, as [`refuse_making`] allows, and narrows an existing one this
/// user owns to owner-only. One another user owns is refused, since its
/// owner could read or replace everything kept there, and so is a link to
/// it another user made, which they could later point elsewhere.
///
/// # Errors
/// Returns an error of kind [`io::ErrorKind::PermissionDenied`] naming the
/// directory when another user owns it or its link, or may not have it
/// made, of kind [`io::ErrorKind::NotFound`] when it is the old place a move
/// took away, or the I/O error from creating, inspecting or narrowing it.
pub fn prepare_state_dir(state_dir: &Path) -> io::Result<()> {
    let user = geteuid().as_raw();
    refuse_making(state_dir, user, legacy_state_dir().as_deref())?;
    create_private_dir_all(state_dir)?;
    let link = fs::symlink_metadata(state_dir)?;
    if link.is_symlink() && link.uid() != user {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "the state directory {} is a link another user (uid {}) made; muster keeps its \
                 state only where you alone decide",
                state_dir.display(),
                link.uid()
            ),
        ));
    }
    let metadata = fs::metadata(state_dir)?;
    if metadata.uid() != user {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "the state directory {} belongs to another user (uid {}); muster keeps it private to its owner",
                state_dir.display(),
                metadata.uid()
            ),
        ));
    }
    if metadata.permissions().mode() & GROUP_OTHER_BITS != 0 {
        fs::set_permissions(state_dir, Permissions::from_mode(PRIVATE_DIR_MODE))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{os::unix::fs::symlink, process};

    use tempfile::TempDir;

    use super::*;

    /// A directory mode other users can enter.
    const SHARED_DIR_MODE: u32 = 0o755;
    /// A directory root owns.
    const ROOT_OWNED_DIR: &str = "/";
    /// A folder no test makes.
    const ABSENT_DIR: &str = "muster-absent";
    /// A shared folder with the sticky bit set.
    const STICKY_SHARED_DIR: &str = "/tmp";

    /// The permission bits of `dir`.
    fn mode_of(dir: &Path) -> u32 {
        fs::metadata(dir).unwrap().permissions().mode() & PERMISSION_BITS
    }

    /// A state directory the service creates is private to its owner, down
    /// to any parent it had to create.
    #[test]
    fn a_created_state_dir_is_private() {
        let scratch = TempDir::new().unwrap();
        let state_dir = scratch.path().join("state/muster");

        prepare_state_dir(&state_dir).unwrap();

        for dir in [state_dir.as_path(), scratch.path().join("state").as_path()] {
            assert_eq!(mode_of(dir), PRIVATE_DIR_MODE, "{}", dir.display());
        }
    }

    /// An existing state directory other users can enter is narrowed to its
    /// owner.
    #[test]
    fn an_existing_shared_state_dir_is_made_private() {
        let scratch = TempDir::new().unwrap();
        let state_dir = scratch.path().join("muster");
        fs::create_dir(&state_dir).unwrap();
        fs::set_permissions(&state_dir, Permissions::from_mode(SHARED_DIR_MODE)).unwrap();

        prepare_state_dir(&state_dir).unwrap();

        assert_eq!(mode_of(&state_dir), PRIVATE_DIR_MODE);
    }

    /// A socket directory the service creates is private to its owner.
    #[test]
    fn a_created_socket_dir_is_private() {
        let scratch = TempDir::new().unwrap();
        let dir = scratch.path().join("run/muster");

        prepare_socket_dir(&dir).unwrap();

        assert_eq!(mode_of(&dir), PRIVATE_DIR_MODE);
    }

    /// An existing socket directory other users can enter, or a link in its
    /// place, is refused rather than narrowed or followed.
    #[test]
    fn a_socket_dir_that_is_not_private_is_refused() {
        let scratch = TempDir::new().unwrap();
        let shared = scratch.path().join("shared");
        fs::create_dir(&shared).unwrap();
        fs::set_permissions(&shared, Permissions::from_mode(SHARED_DIR_MODE)).unwrap();
        let private = scratch.path().join("private");
        fs::create_dir(&private).unwrap();
        let link = scratch.path().join("link");
        symlink(&private, &link).unwrap();

        for dir in [&shared, &link] {
            let error = prepare_socket_dir(dir).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::PermissionDenied, "{error}");
            assert!(
                error.to_string().contains(&*dir.to_string_lossy()),
                "{error}"
            );
        }
        assert_eq!(mode_of(&shared), SHARED_DIR_MODE);
    }

    /// A socket directory that already exists but another user owns is
    /// refused, naming it, even with no group or other bits to narrow.
    #[test]
    fn a_socket_dir_another_user_owns_is_refused() {
        if geteuid().is_root() {
            return;
        }
        let root = Path::new(ROOT_OWNED_DIR);

        for error in [
            prepare_socket_dir(root).unwrap_err(),
            check_socket_dir(root).unwrap_err(),
        ] {
            assert_eq!(error.kind(), io::ErrorKind::PermissionDenied, "{error}");
            assert!(error.to_string().contains("directory / "), "{error}");
        }
    }

    /// A state directory another user owns is refused, naming it, and left
    /// as it is.
    #[test]
    fn a_state_dir_another_user_owns_is_refused() {
        if geteuid().is_root() {
            return;
        }
        let root = Path::new(ROOT_OWNED_DIR);
        let error = prepare_state_dir(root).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert!(error.to_string().contains("another user"), "{error}");
    }

    /// The old place a move took away is never made again, so what resolved
    /// it before the move resolves again instead of using an empty folder.
    #[test]
    fn the_moved_legacy_dir_is_never_made_again() {
        let scratch = TempDir::new().unwrap();
        let moved = scratch.path().join("muster");
        let user = geteuid().as_raw();

        let error = refuse_making(&moved, user, Some(&moved)).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::NotFound, "{error}");
        assert!(!moved.exists());
        refuse_making(&moved, user, None).unwrap();
    }

    /// A state directory is not made inside a folder another user owns, as
    /// root would in a shell that kept the user's home; inside a sticky
    /// shared folder, such as `/tmp`, it is.
    #[test]
    fn a_state_dir_is_not_made_inside_another_users_folder() {
        if geteuid().is_root() {
            return;
        }
        let user = geteuid().as_raw();
        let theirs = Path::new(ROOT_OWNED_DIR).join(ABSENT_DIR).join(APP_DIR);

        let error = refuse_making(&theirs, user, None).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied, "{error}");
        assert!(error.to_string().contains("another user"), "{error}");
        let shared = Path::new(STICKY_SHARED_DIR)
            .join(format!("{ABSENT_DIR}-{}", process::id()))
            .join(APP_DIR);
        refuse_making(&shared, user, None).unwrap();
    }

    /// A state directory that is this user's own link to a folder of theirs
    /// is used.
    #[test]
    fn a_state_dir_linked_by_its_owner_is_used() {
        let scratch = TempDir::new().unwrap();
        let real = scratch.path().join("real");
        fs::create_dir(&real).unwrap();
        let link = scratch.path().join("muster");
        symlink(&real, &link).unwrap();

        prepare_state_dir(&link).unwrap();
    }

    /// A folder is this user's only when this user owns it; the same folder
    /// seen as another user is not.
    #[test]
    fn a_dir_is_claimed_only_by_its_owner() {
        let scratch = TempDir::new().unwrap();
        let user = geteuid().as_raw();

        assert!(claimed_by_effective_user(scratch.path()));
        assert!(claimed_by(scratch.path(), user));
        assert!(!claimed_by(scratch.path(), user + 1));
        if !geteuid().is_root() {
            assert!(!claimed_by_effective_user(Path::new(ROOT_OWNED_DIR)));
        }
    }

    /// CR-6: an absent folder is made owner-only and claimed when this user
    /// owns its parent; as another user, or under a missing parent, nothing
    /// is made.
    #[test]
    fn an_absent_dir_is_made_only_under_a_parent_of_this_user() {
        let scratch = TempDir::new().unwrap();
        let user = geteuid().as_raw();
        let absent = scratch.path().join("muster");

        assert!(!claimed_by(&absent, user + 1));
        assert!(!absent.exists(), "made as another user");
        let orphan = scratch.path().join("missing/muster");
        assert!(!claimed_by(&orphan, user));
        assert!(
            !scratch.path().join("missing").exists(),
            "a parent was made"
        );

        assert!(claimed_by(&absent, user));
        assert_eq!(mode_of(&absent), PRIVATE_DIR_MODE);
    }
}
