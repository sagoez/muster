//! Directories only their owner can enter, for everything muster keeps about
//! a user's processes: logs, records, sockets and shared state.

use std::{
    env,
    fs::{self, DirBuilder, Permissions},
    io,
    os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
};

use directories::ProjectDirs;
use muster_domain::constants::APP_NAME;
use rustix::process::geteuid;

use super::{dir_refusal::DirRefusal, no_state_dir::NoStateDir};

/// Application directory name under the platform's state location.
const APP_DIR: &str = APP_NAME;

/// Mode of a private directory: its owner's alone.
pub const PRIVATE_DIR_MODE: u32 = 0o700;
/// Mode bits granting the group or other users any access.
pub const GROUP_OTHER_BITS: u32 = 0o077;
/// The permission bits of a mode.
const PERMISSION_BITS: u32 = 0o777;
/// Variable naming the user's runtime directory, where the service socket
/// goes; set it to a shorter folder when the socket path does not fit.
pub const RUNTIME_DIR_ENV: &str = "XDG_RUNTIME_DIR";
/// Short folder the socket directory goes in when no runtime directory is
/// set, as on macOS, and where every holder socket goes.
pub const FALLBACK_SOCKET_ROOT: &str = "/tmp";
/// Name of the fallback socket directory, before the user's uid.
const FALLBACK_SOCKET_PREFIX: &str = "muster-";

/// Muster's platform state directory (`~/.local/state/muster` on Linux),
/// falling back to local data on platforms without a distinct state location.
///
/// # Errors
/// Returns [`NoStateDir`] when the platform offers no such location.
pub fn state_dir() -> Result<PathBuf, NoStateDir> {
    ProjectDirs::from("", "", APP_DIR)
        .map(|dirs| {
            dirs.state_dir()
                .unwrap_or_else(|| dirs.data_local_dir())
                .to_path_buf()
        })
        .ok_or(NoStateDir)
}

/// The short directory the service socket goes in, so its path fits a Unix
/// socket address wherever the state directory is: `muster` in
/// `$XDG_RUNTIME_DIR`, else `/tmp/muster-<uid>`.
pub fn socket_dir() -> PathBuf {
    env::var_os(RUNTIME_DIR_ENV)
        .map(PathBuf::from)
        .filter(|runtime| runtime.is_absolute())
        .map_or_else(tmp_socket_dir, |runtime| runtime.join(APP_DIR))
}

/// `/tmp/muster-<uid>`: always short, and never removed at logout as a
/// runtime directory is, so a socket there outlives the login session.
pub fn tmp_socket_dir() -> PathBuf {
    Path::new(FALLBACK_SOCKET_ROOT).join(format!("{FALLBACK_SOCKET_PREFIX}{}", geteuid().as_raw()))
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
/// with mode 0700, saying what clears the refusal as [`DirRefusal`] does.
/// Another user's folder other than `/tmp/muster-<uid>` can also be avoided
/// by moving the service socket, which the error offers too.
///
/// # Errors
/// Returns an error of kind [`io::ErrorKind::PermissionDenied`] naming the
/// directory when it is not private to this user, or the I/O error from
/// inspecting it, of kind [`io::ErrorKind::NotFound`] when it is absent.
pub fn check_socket_dir(dir: &Path) -> io::Result<()> {
    if is_owned_private_dir(dir)? {
        return Ok(());
    }
    let refusal = DirRefusal::of(dir);
    let mut message = format!(
        "the service socket directory {} is not a directory owned by you with mode \
         {PRIVATE_DIR_MODE:04o}, so muster will not use a socket there; {}",
        dir.display(),
        refusal.remedy(dir)
    );
    if refusal.is_foreign() && dir != tmp_socket_dir() {
        message.push_str(&format!(
            "; or set {RUNTIME_DIR_ENV} to a directory of your own to move the service socket"
        ));
    }
    Err(io::Error::new(io::ErrorKind::PermissionDenied, message))
}

/// Whether `uid` is this process's effective user.
pub fn is_current_user(uid: u32) -> bool {
    uid == geteuid().as_raw()
}

/// Whether `dir` is a real directory, not a link, this user owns with mode
/// 0700: the one rule every socket folder muster uses follows, the service's
/// and the holders'.
///
/// # Errors
/// Returns the I/O error from inspecting it, of kind
/// [`io::ErrorKind::NotFound`] when it is absent.
pub fn is_owned_private_dir(dir: &Path) -> io::Result<bool> {
    let metadata = fs::symlink_metadata(dir)?;
    Ok(metadata.is_dir()
        && metadata.uid() == geteuid().as_raw()
        && metadata.permissions().mode() & PERMISSION_BITS == PRIVATE_DIR_MODE)
}

/// The uid that owns whatever is at `dir`, not following a link, when it
/// can be read.
pub fn owner_of(dir: &Path) -> Option<u32> {
    fs::symlink_metadata(dir)
        .ok()
        .map(|metadata| metadata.uid())
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
/// shell that kept the user's home, leaves it alone.
pub fn claimed_by_effective_user(dir: &Path) -> bool {
    claimed_by(dir, geteuid().as_raw())
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
/// owner-only, and narrows an existing one this user owns to owner-only. One
/// another user owns is refused, since its owner could read or replace
/// everything kept there.
///
/// # Errors
/// Returns an error of kind [`io::ErrorKind::PermissionDenied`] naming the
/// directory when another user owns it, or the I/O error from creating,
/// inspecting or narrowing it.
pub fn prepare_state_dir(state_dir: &Path) -> io::Result<()> {
    create_private_dir_all(state_dir)?;
    let metadata = fs::metadata(state_dir)?;
    let user = geteuid().as_raw();
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
    use std::os::unix::fs::symlink;

    use tempfile::TempDir;

    use super::*;

    /// A directory mode other users can enter.
    const SHARED_DIR_MODE: u32 = 0o755;
    /// A directory mode only its owner can enter, but not write.
    const READ_ONLY_DIR_MODE: u32 = 0o500;
    /// A directory root owns.
    const ROOT_OWNED_DIR: &str = "/";

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

    /// Only a real directory this user owns that no other user can enter is
    /// an owned private one: not a shared one, a link to a private one, a
    /// file, or one another user owns.
    #[test]
    fn an_owned_private_dir_is_real_owned_and_closed() {
        let scratch = TempDir::new().unwrap();
        let private = scratch.path().join("private");
        create_private_dir_all(&private).unwrap();
        let shared = scratch.path().join("shared");
        fs::create_dir(&shared).unwrap();
        fs::set_permissions(&shared, Permissions::from_mode(SHARED_DIR_MODE)).unwrap();
        let link = scratch.path().join("link");
        symlink(&private, &link).unwrap();
        let file = scratch.path().join("file");
        fs::write(&file, b"").unwrap();
        let narrower = scratch.path().join("narrower");
        fs::create_dir(&narrower).unwrap();
        fs::set_permissions(&narrower, Permissions::from_mode(READ_ONLY_DIR_MODE)).unwrap();

        assert!(is_owned_private_dir(&private).unwrap());
        assert!(
            !is_owned_private_dir(&narrower).unwrap(),
            "the service's and the holders' folders follow one rule: mode 0700"
        );
        for refused in [&shared, &link, &file] {
            assert!(
                !is_owned_private_dir(refused).unwrap(),
                "{}",
                refused.display()
            );
        }
        if !geteuid().is_root() {
            assert!(!is_owned_private_dir(Path::new(ROOT_OWNED_DIR)).unwrap());
        }
        let absent = is_owned_private_dir(&scratch.path().join("absent")).unwrap_err();
        assert_eq!(absent.kind(), io::ErrorKind::NotFound);
        assert_eq!(owner_of(&private), Some(geteuid().as_raw()));
        assert_eq!(owner_of(&scratch.path().join("absent")), None);
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

    /// A refused service socket folder of one's own, as a runtime folder
    /// made with the wrong mode, is fixed with `chmod`: the error never
    /// sends the user to remove `/tmp/muster-<uid>`, to reboot or to move
    /// the socket.
    #[test]
    fn a_refused_socket_dir_of_ones_own_says_to_chmod_it() {
        let scratch = TempDir::new().unwrap();
        let shared = scratch.path().join("shared");
        fs::create_dir(&shared).unwrap();
        fs::set_permissions(&shared, Permissions::from_mode(SHARED_DIR_MODE)).unwrap();

        let error = check_socket_dir(&shared).unwrap_err().to_string();

        assert!(
            error.contains(&format!("chmod 700 {}", shared.display())),
            "{error}"
        );
        for absent in [
            &*tmp_socket_dir().to_string_lossy(),
            "reboot",
            "administrator",
            RUNTIME_DIR_ENV,
        ] {
            assert!(!error.contains(absent), "{error}");
        }
    }

    /// Another user's service socket folder outside `/tmp` is theirs to
    /// remove, or avoided by moving the socket, with no word of a reboot or
    /// of `/tmp/muster-<uid>`.
    #[test]
    fn another_users_socket_dir_offers_moving_the_socket() {
        if geteuid().is_root() {
            return;
        }

        let error = check_socket_dir(Path::new(ROOT_OWNED_DIR))
            .unwrap_err()
            .to_string();

        assert!(
            error.contains("administrator") && error.contains(RUNTIME_DIR_ENV),
            "{error}"
        );
        assert!(
            !error.contains("reboot") && !error.contains(&*tmp_socket_dir().to_string_lossy()),
            "{error}"
        );
    }

    /// Only this process's own uid is the current user.
    #[test]
    fn the_current_user_is_the_effective_uid() {
        assert!(is_current_user(geteuid().as_raw()));
        assert!(!is_current_user(geteuid().as_raw().wrapping_add(1)));
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
