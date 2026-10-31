//! Reaching an agent's config file without trusting what is at its path: a
//! path is resolved by hand, so a link that leads nowhere is still written
//! at its target; a project's file must stay in the project; and only a
//! regular file of bounded size is read.

use std::{
    fs, io,
    os::unix::fs::PermissionsExt,
    path::{self, Component, Path, PathBuf},
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

/// The file `path` leads to, fully resolved: every link on the way followed,
/// in its folders as in the file itself, and every `..` taken after the
/// link before it, so the result holds neither. A part that does not exist
/// yet is kept as named, and a `..` after it steps back over it, as making
/// the folders and then the file would. A link leading nowhere gives where
/// it points, so the file is made there and the link kept.
///
/// # Errors
/// Returns [`AgentConfigError::Io`] for a link that cannot be read, or more
/// links than the kernel follows.
pub(super) fn resolved(path: &Path) -> Result<PathBuf, AgentConfigError> {
    let absolute = path::absolute(path).map_err(|source| io_error(FOLLOW, path, source))?;
    let mut pending = parts(&absolute);
    let mut current = PathBuf::new();
    let mut hops = 0;
    while let Some(part) = pending.pop() {
        match part.components().next() {
            Some(Component::RootDir) => current = part,
            Some(Component::ParentDir) => {
                current.pop();
            },
            Some(Component::Normal(name)) => {
                current.push(name);
                let is_link =
                    fs::symlink_metadata(&current).is_ok_and(|meta| meta.file_type().is_symlink());
                if is_link {
                    hops += 1;
                    if hops > MAX_LINK_HOPS {
                        return Err(io_error(FOLLOW, path, Errno::LOOP.into()));
                    }
                    let target =
                        fs::read_link(&current).map_err(|source| io_error(FOLLOW, path, source))?;
                    current.pop();
                    pending.extend(parts(&target));
                }
            },
            Some(Component::CurDir | Component::Prefix(_)) | None => {},
        }
    }
    Ok(current)
}

/// The parts of `path`, each its own path, last first so the next one to
/// take is popped off the end.
fn parts(path: &Path) -> Vec<PathBuf> {
    path.components()
        .rev()
        .map(|part| PathBuf::from(part.as_os_str()))
        .collect()
}

/// Refuses `written`, the file a project's config at `path` resolves to,
/// when it lies outside the project at `root`. `written` comes from
/// [`resolved`], so it holds no link or `..` that could lead out after the
/// check.
///
/// # Errors
/// Returns [`AgentConfigError::OutsideProject`] for a file outside the
/// project, or [`AgentConfigError::Io`] for a root that cannot be resolved.
pub(super) fn confine(written: &Path, root: &Path, path: &Path) -> Result<(), AgentConfigError> {
    let root = fs::canonicalize(root).map_err(|source| io_error(FOLLOW, root, source))?;
    if written.starts_with(&root) && written != root {
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

    /// A scratch folder and its path with every link followed.
    fn scratch() -> (TempDir, PathBuf) {
        let dir = TempDir::new().unwrap();
        let base = dir.path().canonicalize().unwrap();
        (dir, base)
    }

    /// A chain of links ends at its last target, a dangling one at where it
    /// points, a linked folder at the file inside its target, and a loop is
    /// refused.
    #[test]
    fn a_link_is_followed_to_its_end_even_when_it_dangles() {
        let (_dir, base) = scratch();
        let real = base.join("dots/real.json");
        fs::create_dir_all(real.parent().unwrap()).unwrap();
        fs::write(&real, "{}").unwrap();
        symlink("dots/real.json", base.join("middle")).unwrap();
        symlink("middle", base.join("config.json")).unwrap();
        assert_eq!(resolved(&base.join("config.json")).unwrap(), real);

        symlink("dots/missing.json", base.join("dangling")).unwrap();
        assert_eq!(
            resolved(&base.join("dangling")).unwrap(),
            base.join("dots/missing.json")
        );
        assert_eq!(resolved(&real).unwrap(), real);
        symlink("dots", base.join("linked")).unwrap();
        assert_eq!(resolved(&base.join("linked/./real.json")).unwrap(), real);

        symlink("loop", base.join("loop")).unwrap();
        assert!(matches!(
            resolved(&base.join("loop")),
            Err(AgentConfigError::Io { .. })
        ));
    }

    /// A `..` after a link steps back from where the link leads, and one
    /// after a folder not made yet steps back over it, so neither hides a
    /// path out of the project behind a missing folder.
    #[test]
    fn every_dot_dot_is_taken_after_the_link_before_it() {
        let (_dir, base) = scratch();
        let root = base.join("deep/project");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(base.join("elsewhere/inner")).unwrap();
        symlink("../../elsewhere/inner", root.join("linked")).unwrap();
        assert_eq!(
            resolved(&root.join("linked/../x.json")).unwrap(),
            base.join("elsewhere/x.json")
        );
        assert_eq!(
            resolved(&root.join("missing/../../../victim/config.toml")).unwrap(),
            base.join("victim/config.toml")
        );
    }

    /// A chain as long as the kernel follows is followed to its end; one
    /// link more is refused.
    #[test]
    fn a_chain_of_the_most_links_is_followed() {
        let (_dir, base) = scratch();
        let real = base.join("real.json");
        fs::write(&real, "{}").unwrap();
        let link = |hop: usize| base.join(format!("link-{hop}"));
        symlink(&real, link(1)).unwrap();
        for hop in 2..=MAX_LINK_HOPS + 1 {
            symlink(link(hop - 1), link(hop)).unwrap();
        }
        assert_eq!(resolved(&link(MAX_LINK_HOPS)).unwrap(), real);
        assert!(matches!(
            resolved(&link(MAX_LINK_HOPS + 1)),
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
    /// file or folder outside it, or a `..` past a missing folder, is
    /// refused.
    #[test]
    fn a_project_file_must_stay_in_the_project() {
        let (_dir, base) = scratch();
        let root = base.join("project");
        let outside = base.join("outside");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&outside).unwrap();
        let confined = |path: &Path| confine(&resolved(path).unwrap(), &root, path);

        let inside = root.join(".codex/config.toml");
        assert!(confined(&inside).is_ok());
        assert!(confined(&root.join("missing/../.mcp.json")).is_ok());
        symlink(&outside, root.join(".codex")).unwrap();
        assert!(matches!(
            confined(&inside),
            Err(AgentConfigError::OutsideProject { .. })
        ));
        let escaping = root.join(".mcp.json");
        symlink("missing/../../outside/x.json", &escaping).unwrap();
        assert!(matches!(
            confined(&escaping),
            Err(AgentConfigError::OutsideProject { .. })
        ));
        assert!(matches!(
            confined(&root.join("missing/..")),
            Err(AgentConfigError::OutsideProject { .. })
        ));
    }
}
