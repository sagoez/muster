//! Directories only their owner can enter, for everything muster keeps about
//! a user's processes: logs, records, sockets and shared state.

use std::{
    env,
    fs::{self, DirBuilder, Metadata, Permissions},
    io,
    os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
};

use muster_domain::constants::APP_NAME;
use rustix::process::{getegid, geteuid};

use super::{
    no_state_dir::NoStateDir,
    path::{is_absent, is_real_dir},
    state_location::{STATE_HOME_ENV, StateLocation},
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
/// Mode bit letting every user write a folder.
const WORLD_WRITE_BIT: u32 = 0o002;
/// Mode bit letting a folder's group write it.
const GROUP_WRITE_BIT: u32 = 0o020;
/// The superuser's uid, whose folders, as `/var` is, stand above many a
/// user's.
const ROOT_UID: u32 = 0;
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
    StateLocation::current().map(|location| location.in_use())
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
/// shell that kept the user's home, leaves it alone. Nothing is made where
/// [`refuse_making`] refuses to, given `legacy`, the place an earlier muster
/// kept its state in.
pub fn claimed_by_effective_user(dir: &Path, legacy: Option<&Path>) -> bool {
    let user = geteuid().as_raw();
    refuse_making(dir, user, legacy).is_ok() && claimed_by(dir, user)
}

/// Refuses to make the absent state directory `dir` as the user `uid`: when
/// it is `legacy`, the old place a move took away, or `legacy` is still a
/// real directory, whose state a move has not brought here yet, so whatever
/// resolved `dir` meanwhile resolves again instead of making an empty one;
/// or when the nearest folder above it that exists belongs to another user
/// and is not sticky, as a user's home is to root.
///
/// # Errors
/// Returns an error of kind [`io::ErrorKind::NotFound`] while the state is
/// moving and of kind [`io::ErrorKind::PermissionDenied`] for another
/// user's folder, each naming the directory.
pub fn refuse_making(dir: &Path, uid: u32, legacy: Option<&Path>) -> io::Result<()> {
    if !is_absent(dir) {
        return Ok(());
    }
    if let Some(legacy) = legacy.filter(|legacy| *legacy == dir || is_real_dir(legacy)) {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!(
                "the state directory {} is moving from {}; run the command again",
                dir.display(),
                legacy.display()
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

/// Whether the user `uid` owns `dir`, creating it and its missing parents
/// owner-only first when it is absent and `uid` owns the nearest folder
/// above it that exists, as a fresh home's `~/.local/state` is missing. A
/// directory another user owns, or made meanwhile, is not claimed, and
/// nothing is made under another user's folder.
pub fn claimed_by(dir: &Path, uid: u32) -> bool {
    if fs::symlink_metadata(dir).is_err() {
        let above = dir
            .ancestors()
            .skip(1)
            .find(|folder| fs::metadata(folder).is_ok());
        if !above.is_some_and(|folder| owned_by(folder, uid)) {
            return false;
        }
        let _ = create_private_dir_all(dir);
    }
    owned_by(dir, uid)
}

/// Whether `dir` exists and the user `uid` owns it.
fn owned_by(dir: &Path, uid: u32) -> bool {
    fs::metadata(dir).is_ok_and(|metadata| metadata.uid() == uid)
}

/// Refuses the state directory `state_dir` when it is a link the user `uid`
/// did not make, since whoever made it could later point it elsewhere.
///
/// # Errors
/// Returns an error of kind [`io::ErrorKind::PermissionDenied`] naming the
/// link, or the I/O error from inspecting it.
fn refuse_foreign_link(state_dir: &Path, uid: u32) -> io::Result<()> {
    let link = fs::symlink_metadata(state_dir)?;
    if !link.is_symlink() || link.uid() == uid {
        return Ok(());
    }
    Err(io::Error::new(
        io::ErrorKind::PermissionDenied,
        format!(
            "the state directory {} is a link another user (uid {}) made; muster keeps its state \
             only where you alone decide",
            state_dir.display(),
            link.uid()
        ),
    ))
}

/// Readies muster's state directory: creates it and any missing parents
/// owner-only, as [`refuse_making`] allows given `legacy`, the place an
/// earlier muster kept its state in, refuses it as [`check_state_dir`]
/// does, and narrows it to owner-only.
///
/// # Errors
/// Returns an error of kind [`io::ErrorKind::PermissionDenied`] naming the
/// directory when it may not be made or used, of kind
/// [`io::ErrorKind::NotFound`] while the state moves, or the I/O error from
/// creating, inspecting or narrowing it.
pub fn prepare_state_dir(state_dir: &Path, legacy: Option<&Path>) -> io::Result<()> {
    let user = geteuid().as_raw();
    refuse_making(state_dir, user, legacy)?;
    create_private_dir_all(state_dir)?;
    check_state_dir(state_dir, user)?;
    if fs::metadata(state_dir)?.permissions().mode() & GROUP_OTHER_BITS != 0 {
        fs::set_permissions(state_dir, Permissions::from_mode(PRIVATE_DIR_MODE))?;
    }
    Ok(())
}

/// Refuses the existing state directory `state_dir` for the user `uid`,
/// making nothing: one another user owns, since its owner could read or
/// replace everything kept there; a link another user made, which they
/// could later point elsewhere; and one in a folder that neither `uid` nor
/// root owns, or that others may write, since they could swap it.
///
/// # Errors
/// Returns an error of kind [`io::ErrorKind::PermissionDenied`] naming the
/// directory or its folder, or the I/O error from inspecting them.
pub fn check_state_dir(state_dir: &Path, uid: u32) -> io::Result<()> {
    refuse_foreign_link(state_dir, uid)?;
    let metadata = fs::metadata(state_dir)?;
    if metadata.uid() != uid {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "the state directory {} belongs to another user (uid {}); muster keeps it \
                 private to its owner",
                state_dir.display(),
                metadata.uid()
            ),
        ));
    }
    let Some(folder) = state_dir.parent() else {
        return Ok(());
    };
    let above = fs::metadata(folder)?;
    let owner_trusted = above.uid() == uid || above.uid() == ROOT_UID;
    if owner_trusted && !writable_by_others(&above) {
        return Ok(());
    }
    Err(unsafe_folder(state_dir, folder))
}

/// Refuses `folder`, the folder a move puts the state directory in, unless
/// it is a real directory, not a link, that the user `uid` owns and no
/// other user may write.
///
/// # Errors
/// Returns an error of kind [`io::ErrorKind::PermissionDenied`] naming the
/// folder, or the I/O error from inspecting it.
pub fn check_move_folder(state_dir: &Path, uid: u32) -> io::Result<()> {
    let Some(folder) = state_dir.parent() else {
        return Ok(());
    };
    let metadata = fs::symlink_metadata(folder)?;
    if metadata.is_dir() && metadata.uid() == uid && !writable_by_others(&metadata) {
        return Ok(());
    }
    Err(unsafe_folder(state_dir, folder))
}

/// Whether users besides the owner may add, rename or remove entries in
/// the folder `metadata` describes: it is writable by everyone, or by a
/// group other than this process's own, and not sticky.
fn writable_by_others(metadata: &Metadata) -> bool {
    let mode = metadata.mode();
    let by_world = mode & WORLD_WRITE_BIT != 0;
    let by_group = mode & GROUP_WRITE_BIT != 0 && metadata.gid() != getegid().as_raw();
    (by_world || by_group) && mode & STICKY_BIT == 0
}

/// The refusal of `state_dir` for lying in `folder`, which another user
/// owns or may write.
fn unsafe_folder(state_dir: &Path, folder: &Path) -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        format!(
            "muster will not keep its state in {}: {} is a folder another user owns or may \
             write; set {STATE_HOME_ENV} to a folder of your own",
            state_dir.display(),
            folder.display()
        ),
    )
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
    /// A folder a missing state directory's parent would be.
    const MISSING_PARENT: &str = "missing";
    /// Where an earlier muster kept its state, in a scratch folder.
    const LEGACY_DIR: &str = "old-muster";
    /// A folder a test opens to other users.
    const SHARED_FOLDER: &str = "shared";
    /// A link to a folder.
    const LINKED_FOLDER: &str = "linked";
    /// A folder mode every user may write, not sticky.
    const OPEN_DIR_MODE: u32 = 0o777;

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

        prepare_state_dir(&state_dir, None).unwrap();

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

        prepare_state_dir(&state_dir, None).unwrap();

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
        let error = prepare_state_dir(root, None).unwrap_err();

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

        prepare_state_dir(&link, None).unwrap();
    }

    /// A state directory that is a link another user made is refused,
    /// naming it, even when it leads to a folder of this user's.
    #[test]
    fn a_state_dir_linked_by_another_user_is_refused() {
        let scratch = TempDir::new().unwrap();
        let real = scratch.path().join("real");
        fs::create_dir(&real).unwrap();
        let link = scratch.path().join("muster");
        symlink(&real, &link).unwrap();
        let other = geteuid().as_raw() + 1;

        let error = refuse_foreign_link(&link, other).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied, "{error}");
        assert!(
            error.to_string().contains(&*link.to_string_lossy()),
            "{error}"
        );
        refuse_foreign_link(&real, other).unwrap();
    }

    /// A folder is this user's only when this user owns it; the same folder
    /// seen as another user is not.
    #[test]
    fn a_dir_is_claimed_only_by_its_owner() {
        let scratch = TempDir::new().unwrap();
        let user = geteuid().as_raw();

        assert!(claimed_by_effective_user(scratch.path(), None));
        assert!(claimed_by(scratch.path(), user));
        assert!(!claimed_by(scratch.path(), user + 1));
        if !geteuid().is_root() {
            assert!(!claimed_by_effective_user(Path::new(ROOT_OWNED_DIR), None));
        }
    }

    /// CR-6: an absent folder is made owner-only and claimed, with any
    /// missing parents, as a fresh home's `~/.local/state` is, when this
    /// user owns the nearest folder above it; as another user nothing is
    /// made.
    #[test]
    fn an_absent_dir_is_made_only_under_a_folder_of_this_user() {
        let scratch = TempDir::new().unwrap();
        let user = geteuid().as_raw();
        let absent = scratch.path().join("muster");

        assert!(!claimed_by(&absent, user + 1));
        assert!(!absent.exists(), "made as another user");
        let missing = scratch.path().join(MISSING_PARENT);
        assert!(!claimed_by(&missing.join(APP_DIR), user + 1));
        assert!(!missing.exists(), "a parent was made as another user");

        assert!(claimed_by(&absent, user));
        assert_eq!(mode_of(&absent), PRIVATE_DIR_MODE);
        assert!(claimed_by(&missing.join(APP_DIR), user));
        assert_eq!(mode_of(&missing), PRIVATE_DIR_MODE);
    }

    /// The place a move took the state from is never claimed, and so never
    /// made again; nor is the state directory while the old one still
    /// holds the state.
    #[test]
    fn the_moved_legacy_dir_is_never_claimed() {
        let scratch = TempDir::new().unwrap();
        let moved = scratch.path().join(APP_DIR);

        assert!(!claimed_by_effective_user(&moved, Some(&moved)));
        assert!(!moved.exists());

        let legacy = scratch.path().join(LEGACY_DIR);
        fs::create_dir(&legacy).unwrap();
        assert!(!claimed_by_effective_user(&moved, Some(&legacy)));
        assert!(!moved.exists(), "made while the old one holds the state");
        assert!(claimed_by_effective_user(&moved, None));
    }

    /// A state directory is not made while the old one is still a real
    /// directory, whose state has not moved yet, as after a move went back.
    #[test]
    fn a_state_dir_is_not_made_while_the_old_one_holds_the_state() {
        let scratch = TempDir::new().unwrap();
        let legacy = scratch.path().join(LEGACY_DIR);
        fs::create_dir(&legacy).unwrap();
        let state_dir = scratch.path().join(APP_DIR);

        let error = prepare_state_dir(&state_dir, Some(&legacy)).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::NotFound, "{error}");
        assert!(!state_dir.exists());
    }

    /// A state directory in a folder every user may write, not sticky, is
    /// refused naming the folder, so no one else can swap it.
    #[test]
    fn a_state_dir_in_a_folder_others_write_is_refused() {
        let scratch = TempDir::new().unwrap();
        let shared = scratch.path().join(SHARED_FOLDER);
        fs::create_dir(&shared).unwrap();
        let state_dir = shared.join(APP_DIR);
        fs::create_dir(&state_dir).unwrap();
        fs::set_permissions(&shared, Permissions::from_mode(OPEN_DIR_MODE)).unwrap();

        let error = check_state_dir(&state_dir, geteuid().as_raw()).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied, "{error}");
        assert!(
            error.to_string().contains(&*shared.to_string_lossy()),
            "{error}"
        );
    }

    /// A move puts the state only in a real folder of this user's that no
    /// one else may write: a folder others write, a link, or one another
    /// user owns is refused.
    #[test]
    fn a_move_goes_only_into_a_private_folder_of_this_user() {
        let scratch = TempDir::new().unwrap();
        let user = geteuid().as_raw();
        let own = scratch.path().join(SHARED_FOLDER);
        fs::create_dir(&own).unwrap();
        check_move_folder(&own.join(APP_DIR), user).unwrap();
        assert!(check_move_folder(&own.join(APP_DIR), user + 1).is_err());

        let link = scratch.path().join(LINKED_FOLDER);
        symlink(&own, &link).unwrap();
        assert!(check_move_folder(&link.join(APP_DIR), user).is_err());

        fs::set_permissions(&own, Permissions::from_mode(OPEN_DIR_MODE)).unwrap();
        let error = check_move_folder(&own.join(APP_DIR), user).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied, "{error}");
    }
}
