//! Directories only their owner can enter, for everything muster keeps about
//! a user's processes: logs, records, sockets and shared state.

use std::{
    fs::{self, DirBuilder, Permissions},
    io,
    os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
};

use directories::ProjectDirs;

/// Application directory name under the platform's state location.
const APP_DIR: &str = "muster";

/// Mode of a private directory: its owner's alone.
pub const PRIVATE_DIR_MODE: u32 = 0o700;
/// Mode bits granting the group or other users any access.
pub const GROUP_OTHER_BITS: u32 = 0o077;

/// Muster's platform state directory (`~/.local/state/muster` on Linux),
/// falling back to local data on platforms without a distinct state location.
pub fn state_dir() -> Option<PathBuf> {
    ProjectDirs::from("", "", APP_DIR).map(|dirs| {
        dirs.state_dir()
            .unwrap_or_else(|| dirs.data_local_dir())
            .to_path_buf()
    })
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
    // SAFETY: `geteuid` takes no arguments, touches no memory and cannot fail.
    let user = unsafe { libc::geteuid() };
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
    use tempfile::TempDir;

    use super::*;

    /// Mode bits a permission check compares.
    const PERMISSION_BITS: u32 = 0o777;
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

    /// A state directory another user owns is refused, naming it, and left
    /// as it is.
    #[test]
    fn a_state_dir_another_user_owns_is_refused() {
        // SAFETY: `geteuid` takes no arguments and touches no memory.
        if unsafe { libc::geteuid() } == 0 {
            return;
        }
        let root = Path::new(ROOT_OWNED_DIR);
        let error = prepare_state_dir(root).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert!(error.to_string().contains("another user"), "{error}");
    }
}
