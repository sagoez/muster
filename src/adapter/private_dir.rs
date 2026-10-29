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
use rustix::process::geteuid;

use super::no_state_dir::NoStateDir;
use crate::constants::APP_NAME;

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
/// Variable naming the state directory's parent; set it to a shorter folder
/// when a holder's socket path does not fit.
pub const STATE_HOME_ENV: &str = "XDG_STATE_HOME";
/// Short folder the socket directory goes in when no runtime directory is
/// set, as on macOS.
const FALLBACK_SOCKET_ROOT: &str = "/tmp";
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
}
