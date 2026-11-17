//! Reading and writing an agent's config files, the one way every caller
//! does it: an absent file reads as none, a link is followed to the file it
//! names and stays a link, and a write is atomic and refused up front when
//! this user could not make it.

use std::{
    fs,
    io::{self, ErrorKind},
    path::{Path, PathBuf},
};

use muster_os::private_dir::create_private_dir_all;
use rustix::fs::{Access, access};

use crate::cli::{mcp::hook_file_error::HookFileError, transfer::text_file::write_atomically};

/// The most links followed to the file a path names, as the kernel allows.
const MAX_LINK_HOPS: usize = 40;
/// Why a chain of links was not followed to its end.
const TOO_MANY_LINKS: &str = "too many levels of symbolic links";

/// The text of the file at `path`; none when it does not exist, a link to
/// nothing included.
///
/// # Errors
/// Returns [`HookFileError::Read`] when it cannot be read.
pub fn read_optional(path: &Path) -> Result<Option<String>, HookFileError> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(source) => Err(HookFileError::Read {
            path: path.to_path_buf(),
            source,
        }),
    }
}

/// The file `path` names once every link is followed, even to a target
/// that does not exist yet, so a linked dotfile stays linked.
///
/// # Errors
/// Returns [`HookFileError::Read`] when a link cannot be read or the chain
/// does not end.
pub fn link_target(path: &Path) -> Result<PathBuf, HookFileError> {
    let failed = |source| HookFileError::Read {
        path: path.to_path_buf(),
        source,
    };
    let mut target = path.to_path_buf();
    for _ in 0..MAX_LINK_HOPS {
        match fs::symlink_metadata(&target) {
            Ok(meta) if meta.file_type().is_symlink() => {
                let link = fs::read_link(&target).map_err(failed)?;
                target = match target.parent() {
                    Some(folder) => folder.join(link),
                    None => link,
                };
            },
            _ => return Ok(target),
        }
    }
    Err(failed(io::Error::other(TOO_MANY_LINKS)))
}

/// Refuses a `target` this user could not replace: an existing file it may
/// not write, or a folder, the nearest one that exists, it may not write
/// the new file into.
///
/// # Errors
/// Returns [`HookFileError::ReadOnly`] naming what refused.
pub fn writable(target: &Path) -> Result<(), HookFileError> {
    let refused = |path: &Path| HookFileError::ReadOnly {
        path: path.to_path_buf(),
    };
    if target.exists() && access(target, Access::WRITE_OK).is_err() {
        return Err(refused(target));
    }
    match target.ancestors().skip(1).find(|folder| folder.exists()) {
        Some(folder) if access(folder, Access::WRITE_OK).is_err() => Err(refused(folder)),
        _ => Ok(()),
    }
}

/// Writes `text` as the file `path` names, through its links, atomically,
/// making its missing folders: owner-only when `private`, as a user's own
/// config folder is, else as any folder, as a project's is.
///
/// # Errors
/// Returns [`HookFileError::ReadOnly`] when this user could not write it,
/// [`HookFileError::Read`] for a link that cannot be followed, and
/// [`HookFileError::Write`] when the folder or file cannot be written.
pub fn write_through(path: &Path, text: &str, private: bool) -> Result<(), HookFileError> {
    let target = link_target(path)?;
    writable(&target)?;
    let failed = |source| HookFileError::Write {
        path: path.to_path_buf(),
        source,
    };
    if let Some(folder) = target.parent() {
        if private {
            create_private_dir_all(folder)
        } else {
            fs::create_dir_all(folder)
        }
        .map_err(failed)?;
    }
    write_atomically(&target, text).map_err(failed)
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::{PermissionsExt, symlink};

    use tempfile::TempDir;

    use super::*;

    /// The mode bits a folder's permissions are compared by.
    const PERMISSION_BITS: u32 = 0o777;
    /// The mode of a folder only its owner can enter.
    const OWNER_ONLY: u32 = 0o700;

    /// A private write makes a missing config folder, such as `~/.codex`,
    /// owner-only, and one already there keeps its mode; any other write
    /// makes its folder as any folder is made, as a project's `.claude` is.
    #[test]
    fn only_a_private_write_makes_its_folder_owner_only() {
        let scratch = TempDir::new().unwrap();
        let home = scratch.path().join("home");
        fs::create_dir_all(&home).unwrap();
        let usual = fs::metadata(&home).unwrap().permissions().mode() & PERMISSION_BITS;

        write_through(&home.join(".codex").join("hooks.json"), "{}\n", true).unwrap();
        write_through(&home.join(".claude").join("settings.json"), "{}\n", false).unwrap();

        let mode = |folder: &str| {
            fs::metadata(home.join(folder))
                .unwrap()
                .permissions()
                .mode()
                & PERMISSION_BITS
        };
        assert_eq!(mode(".codex"), OWNER_ONLY);
        assert_eq!(mode(".claude"), usual, "made as the home folder was");
        assert_eq!(
            fs::metadata(&home).unwrap().permissions().mode() & PERMISSION_BITS,
            usual
        );
    }

    /// A write through a link to a file not made yet makes that file and
    /// keeps the link, and an absent file reads as none.
    #[test]
    fn a_write_through_a_dangling_link_keeps_the_link() {
        let scratch = TempDir::new().unwrap();
        let path = scratch.path().join("config.toml");
        let real = scratch.path().join("dotfiles").join("config.toml");
        symlink(&real, &path).unwrap();

        assert_eq!(read_optional(&path).unwrap(), None);
        write_through(&path, "a = 1\n", false).unwrap();

        assert!(
            fs::symlink_metadata(&path)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read_to_string(&real).unwrap(), "a = 1\n");
        assert_eq!(read_optional(&path).unwrap().as_deref(), Some("a = 1\n"));
    }
}
