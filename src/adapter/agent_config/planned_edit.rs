use std::{
    fs,
    io::{self, Write},
    os::unix::fs::OpenOptionsExt,
    path::PathBuf,
};

use atomic_write_file::AtomicWriteFile;
use getset::{CopyGetters, Getters};
use typed_builder::TypedBuilder;

use super::{
    agent_config_error::AgentConfigError,
    config_file::{io_error, read_text, resolved},
    config_target::ConfigTarget,
    edit_outcome::EditOutcome,
};

/// The mode of a config muster creates: only its owner reads it, as Claude
/// Code keeps the account details it adds to `.claude.json`. A file that
/// exists keeps its own mode.
const NEW_FILE_MODE: u32 = 0o600;
/// What a failed write was doing.
pub(super) const WRITE: &str = "write";
/// What a failed folder creation was doing.
const CREATE_FOLDER_FOR: &str = "create the folder for";

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
    /// The file written: `path` resolved, so a linked dotfile stays a link.
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
    /// Whether the edit writes the file.
    pub(super) fn writes(&self) -> bool {
        self.text.is_some()
    }

    /// Checks the file still holds what the edit was worked out against.
    ///
    /// # Errors
    /// Returns [`AgentConfigError::Changed`] when it does not, or the
    /// failure to read it.
    pub(super) fn check_unchanged(&self) -> Result<(), AgentConfigError> {
        if read_text(&self.written, &self.path)? == self.original {
            Ok(())
        } else {
            Err(AgentConfigError::Changed(self.path.clone()))
        }
    }

    /// The new text written and synced to a temporary file beside the
    /// file, its folder made first where the agent expects muster to make
    /// it; the file itself is untouched until the returned file is
    /// committed, and dropping it removes the temporary file. An edit that
    /// writes nothing stages nothing.
    ///
    /// # Errors
    /// Returns [`AgentConfigError::Io`] when the folder or the temporary
    /// file cannot be made or written.
    pub(super) fn stage(&self) -> Result<Option<AtomicWriteFile>, AgentConfigError> {
        let Some(text) = &self.text else {
            return Ok(None);
        };
        // The folders are made on the path that passed the plan's checks, so
        // a link put in it since must not redirect them.
        if resolved(&self.written)? != self.written {
            return Err(AgentConfigError::Changed(self.path.clone()));
        }
        if self.makes_folder
            && let Some(folder) = self.written.parent()
        {
            fs::create_dir_all(folder)
                .map_err(|source| io_error(CREATE_FOLDER_FOR, &self.path, source))?;
        }
        let write = || -> io::Result<AtomicWriteFile> {
            let mut file = AtomicWriteFile::options()
                .mode(NEW_FILE_MODE)
                .open(&self.written)?;
            file.write_all(text.as_bytes())?;
            file.flush()?;
            file.as_file().sync_all()?;
            Ok(file)
        };
        write()
            .map(Some)
            .map_err(|source| io_error(WRITE, &self.path, source))
    }
}
