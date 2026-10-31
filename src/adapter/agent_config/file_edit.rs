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
    config_file::{io_error, resolved},
    file_fingerprint::FileFingerprint,
};

/// The mode of a config muster creates: only its owner reads it, as Claude
/// Code keeps the account details it adds to `.claude.json`. A file that
/// exists keeps its own mode.
const NEW_FILE_MODE: u32 = 0o600;
/// What a failed write was doing.
pub(super) const WRITE: &str = "write";
/// What a failed look at a file was doing.
const CHECK: &str = "check";
/// What a failed folder creation was doing.
const CREATE_FOLDER_FOR: &str = "create the folder for";

/// The new text of one config file, worked out against the file as read and
/// not yet written.
#[derive(Clone, Debug, PartialEq, Eq, Getters, CopyGetters, TypedBuilder)]
pub(super) struct FileEdit {
    /// The file, as the agent names it.
    #[getset(get = "pub(super)")]
    path: PathBuf,
    /// The file written: `path` resolved, so a linked dotfile stays a link.
    #[getset(get = "pub(super)")]
    written: PathBuf,
    /// The file as it was read, or none when it did not exist.
    fingerprint: Option<FileFingerprint>,
    /// The new text.
    text: String,
    /// Whether the file's folder is made when missing, where the agent
    /// expects muster to make it.
    makes_folder: bool,
    /// Whether Claude Code's lock guards the file.
    #[getset(get_copy = "pub(super)")]
    locked: bool,
}

impl FileEdit {
    /// Checks the file is still the one the edit was worked out against,
    /// by its fingerprint rather than a second read.
    ///
    /// # Errors
    /// Returns [`AgentConfigError::Changed`] when it is not, or
    /// [`AgentConfigError::Io`] when it cannot be looked at.
    pub(super) fn check_unchanged(&self) -> Result<(), AgentConfigError> {
        let now = FileFingerprint::at(&self.written)
            .map_err(|source| io_error(CHECK, &self.path, source))?;
        if now == self.fingerprint {
            Ok(())
        } else {
            Err(AgentConfigError::Changed(self.path.clone()))
        }
    }

    /// The new text written and synced to a temporary file beside the
    /// file, its folder made first where the agent expects muster to make
    /// it; the file itself is untouched until the returned file is
    /// committed, and dropping it removes the temporary file.
    ///
    /// # Errors
    /// Returns [`AgentConfigError::Changed`] when a link now leads the
    /// file's path elsewhere, or [`AgentConfigError::Io`] when the folder or
    /// the temporary file cannot be made or written.
    pub(super) fn stage(&self) -> Result<AtomicWriteFile, AgentConfigError> {
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
            file.write_all(self.text.as_bytes())?;
            file.flush()?;
            file.as_file().sync_all()?;
            Ok(file)
        };
        write().map_err(|source| io_error(WRITE, &self.path, source))
    }
}
