//! Reaching an agent's config file without trusting what is at its path: a
//! link is followed by hand, so one that leads nowhere is still written at
//! its target; a project's file must stay in the project; and only a
//! regular file of bounded size is read.

use std::{
    fs, io,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

use bytesize::ByteSize;
use rustix::io::Errno;

use super::agent_config_error::AgentConfigError;
use crate::adapter::bounded_read::read_bounded;

/// The most an agent config muster reads may hold: well above the largest
/// `.claude.json` a long-time user keeps, and a bound on what a project's
/// file can make muster read.
const MAX_CONFIG_SIZE: ByteSize = ByteSize::mib(128);
/// The most links followed to a file, as the kernel allows.
const MAX_LINK_HOPS: usize = 40;
/// The owner's write bit of a file's mode.
const OWNER_WRITE: u32 = 0o200;
/// What a failed read was doing.
const READ: &str = "read";
/// What a failed symlink resolution was doing.
const FOLLOW: &str = "follow the link";

/// The file `path` leads to: the end of its chain of symlinks, else `path`
/// itself. A link leading nowhere gives where it points, so the file is
/// made there and the link kept.
///
/// # Errors
/// Returns [`AgentConfigError::Io`] for a link that cannot be read, or a
/// chain longer than the kernel follows.
pub(super) fn followed(path: &Path) -> Result<PathBuf, AgentConfigError> {
    let mut current = path.to_path_buf();
    for _ in 0..MAX_LINK_HOPS {
        let is_link =
            fs::symlink_metadata(&current).is_ok_and(|meta| meta.file_type().is_symlink());
        if !is_link {
            return Ok(current);
        }
        let target = fs::read_link(&current).map_err(|source| io_error(FOLLOW, path, source))?;
        current = match current.parent() {
            Some(folder) => folder.join(target),
            None => target,
        };
    }
    Err(io_error(FOLLOW, path, Errno::LOOP.into()))
}

/// Refuses `written`, the file a project's config at `path` leads to, when
/// it lies outside the project at `root`: the deepest folder of it that
/// exists, every link in it followed, must be inside `root`.
///
/// # Errors
/// Returns [`AgentConfigError::OutsideProject`] for a file outside the
/// project, or [`AgentConfigError::Io`] for a folder that cannot be
/// resolved.
pub(super) fn confine(written: &Path, root: &Path, path: &Path) -> Result<(), AgentConfigError> {
    let root = fs::canonicalize(root).map_err(|source| io_error(FOLLOW, root, source))?;
    let Some(existing) = written
        .ancestors()
        .find(|ancestor| fs::symlink_metadata(ancestor).is_ok())
    else {
        return Ok(());
    };
    let real = fs::canonicalize(existing).map_err(|source| io_error(FOLLOW, path, source))?;
    if real.starts_with(&root) {
        Ok(())
    } else {
        Err(AgentConfigError::OutsideProject {
            path: path.to_path_buf(),
            target: written.to_path_buf(),
        })
    }
}

/// The text of the config at `written`, which the agent names `path`, or
/// none when there is no file. Only a regular file of at most
/// [`MAX_CONFIG_SIZE`] is read, opened without blocking so a pipe or device
/// is never waited on.
///
/// # Errors
/// Returns [`AgentConfigError::NotAFile`], [`AgentConfigError::TooLarge`],
/// [`AgentConfigError::Unparseable`] for text that is not UTF-8, or
/// [`AgentConfigError::Io`] when the file cannot be read.
pub(super) fn read_text(written: &Path, path: &Path) -> Result<Option<String>, AgentConfigError> {
    match read_bounded(written, MAX_CONFIG_SIZE.as_u64()) {
        Ok(text) => Ok(Some(text)),
        Err(error) => match error.kind() {
            io::ErrorKind::NotFound => Ok(None),
            io::ErrorKind::InvalidInput => Err(AgentConfigError::NotAFile(path.to_path_buf())),
            io::ErrorKind::FileTooLarge => Err(AgentConfigError::TooLarge {
                path: path.to_path_buf(),
                limit: MAX_CONFIG_SIZE,
            }),
            io::ErrorKind::InvalidData => Err(AgentConfigError::Unparseable {
                path: path.to_path_buf(),
                reason: error.to_string(),
            }),
            _ => Err(io_error(READ, path, error)),
        },
    }
}

/// Whether the file at `written` exists and its owner may not write it.
pub(super) fn is_read_only(written: &Path) -> bool {
    fs::metadata(written).is_ok_and(|meta| meta.permissions().mode() & OWNER_WRITE == 0)
}

/// The failure of `action` on `path`.
pub(super) fn io_error(action: &'static str, path: &Path, source: io::Error) -> AgentConfigError {
    AgentConfigError::Io {
        action,
        path: path.to_path_buf(),
        source,
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::symlink;

    use tempfile::TempDir;

    use super::*;
    use crate::adapter::test_fifo::make_fifo;

    /// A chain of links ends at its last target, a dangling one at where it
    /// points, and a loop is refused.
    #[test]
    fn a_link_is_followed_to_its_end_even_when_it_dangles() {
        let dir = TempDir::new().unwrap();
        let real = dir.path().join("dots/real.json");
        fs::create_dir_all(real.parent().unwrap()).unwrap();
        fs::write(&real, "{}").unwrap();
        symlink("dots/real.json", dir.path().join("middle")).unwrap();
        symlink("middle", dir.path().join("config.json")).unwrap();
        assert_eq!(
            followed(&dir.path().join("config.json")).unwrap(),
            dir.path().join("dots/real.json")
        );

        symlink("dots/missing.json", dir.path().join("dangling")).unwrap();
        assert_eq!(
            followed(&dir.path().join("dangling")).unwrap(),
            dir.path().join("dots/missing.json")
        );
        assert_eq!(followed(&real).unwrap(), real);

        symlink("loop", dir.path().join("loop")).unwrap();
        assert!(matches!(
            followed(&dir.path().join("loop")),
            Err(AgentConfigError::Io { .. })
        ));
    }

    /// A FIFO, a device and a file past the cap are refused without being
    /// waited on or read whole; a missing file reads as none.
    #[test]
    fn only_a_regular_file_within_the_cap_is_read() {
        let dir = TempDir::new().unwrap();
        let fifo = dir.path().join("fifo");
        make_fifo(&fifo);
        assert!(matches!(
            read_text(&fifo, &fifo),
            Err(AgentConfigError::NotAFile(_))
        ));
        let zero = Path::new("/dev/zero");
        assert!(matches!(
            read_text(zero, zero),
            Err(AgentConfigError::NotAFile(_))
        ));
        let missing = dir.path().join("missing");
        assert_eq!(read_text(&missing, &missing).unwrap(), None);
        let large = dir.path().join("large");
        let file = fs::File::create(&large).unwrap();
        file.set_len(MAX_CONFIG_SIZE.as_u64() + 1).unwrap();
        assert!(matches!(
            read_text(&large, &large),
            Err(AgentConfigError::TooLarge { .. })
        ));
    }

    /// A file inside the project, or one not made yet, passes; a link to a
    /// file or folder outside it is refused.
    #[test]
    fn a_project_file_must_stay_in_the_project() {
        let dir = TempDir::new().unwrap();
        let root = dir.path().join("project");
        let outside = dir.path().join("outside");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&outside).unwrap();

        let inside = root.join(".codex/config.toml");
        assert!(confine(&inside, &root, &inside).is_ok());
        symlink(&outside, root.join(".codex")).unwrap();
        assert!(matches!(
            confine(&inside, &root, &inside),
            Err(AgentConfigError::OutsideProject { .. })
        ));
        let linked = root.join(".mcp.json");
        assert!(matches!(
            confine(&outside.join("x.json"), &root, &linked),
            Err(AgentConfigError::OutsideProject { .. })
        ));
    }
}
