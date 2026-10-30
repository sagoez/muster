use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

use atomic_write_file::AtomicWriteFile;
use getset::{CopyGetters, Getters};
use typed_builder::TypedBuilder;

use super::{
    agent_config_error::AgentConfigError, config_target::ConfigTarget, edit_outcome::EditOutcome,
};

/// What a failed read was doing.
const READ: &str = "read";
/// What a failed write was doing.
const WRITE: &str = "write";
/// What a failed folder creation was doing.
const CREATE_FOLDER_FOR: &str = "create the folder for";
/// What a failed symlink resolution was doing.
const FOLLOW: &str = "follow the link";

/// An edit of one agent config, worked out against the file as read and not
/// yet written, so every edit of a command is checked before any is made.
#[derive(Clone, Debug, PartialEq, Eq, Getters, CopyGetters, TypedBuilder)]
pub struct PlannedEdit {
    /// The config edited.
    #[getset(get = "pub")]
    target: ConfigTarget,
    /// The file, as the agent names it.
    #[getset(get = "pub")]
    path: PathBuf,
    /// The file written: `path` with a symlink to it followed, so a linked
    /// dotfile stays a link.
    written: PathBuf,
    /// What the edit does.
    #[getset(get_copy = "pub")]
    outcome: EditOutcome,
    /// The file's text as read, or none when it did not exist.
    original: Option<String>,
    /// The new text, when the edit writes.
    text: Option<String>,
    /// Whether the file's folder is made when missing, where the agent
    /// expects muster to make it.
    makes_folder: bool,
    /// What the agent does with the entry that a person should know.
    #[getset(get_copy = "pub")]
    note: Option<&'static str>,
}

impl PlannedEdit {
    /// Writes the new text atomically, keeping the file's mode, unless the
    /// file changed since it was read; an edit that writes nothing does
    /// nothing.
    ///
    /// # Errors
    /// Returns [`AgentConfigError::Changed`] when the file changed since it
    /// was read, or [`AgentConfigError::Io`] when it cannot be written.
    pub fn commit(&self) -> Result<(), AgentConfigError> {
        let Some(text) = &self.text else {
            return Ok(());
        };
        if self.makes_folder
            && let Some(folder) = self.written.parent()
        {
            fs::create_dir_all(folder)
                .map_err(|source| io_error(CREATE_FOLDER_FOR, &self.path, source))?;
        }
        if read_text(&self.written)? != self.original {
            return Err(AgentConfigError::Changed(self.path.clone()));
        }
        let write = || -> io::Result<()> {
            let mut file = AtomicWriteFile::open(&self.written)?;
            file.write_all(text.as_bytes())?;
            file.commit()
        };
        write().map_err(|source| io_error(WRITE, &self.path, source))
    }
}

/// The file `path` leads to: the target of a symlink, else `path` itself.
///
/// # Errors
/// Returns [`AgentConfigError::Io`] for a symlink that leads nowhere.
pub(super) fn followed(path: &Path) -> Result<PathBuf, AgentConfigError> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => {
            fs::canonicalize(path).map_err(|source| io_error(FOLLOW, path, source))
        },
        _ => Ok(path.to_path_buf()),
    }
}

/// The text of the file at `path`, or none when there is no file.
///
/// # Errors
/// Returns [`AgentConfigError::Io`] when the file cannot be read, and
/// [`AgentConfigError::Unparseable`] when it is not UTF-8.
pub(super) fn read_text(path: &Path) -> Result<Option<String>, AgentConfigError> {
    match fs::read(path) {
        Ok(bytes) => {
            String::from_utf8(bytes)
                .map(Some)
                .map_err(|error| AgentConfigError::Unparseable {
                    path: path.to_path_buf(),
                    reason: error.utf8_error().to_string(),
                })
        },
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(io_error(READ, path, source)),
    }
}

/// The failure of `action` on `path`.
fn io_error(action: &'static str, path: &Path, source: io::Error) -> AgentConfigError {
    AgentConfigError::Io {
        action,
        path: path.to_path_buf(),
        source,
    }
}
