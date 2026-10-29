use std::{io, path::PathBuf};

use thiserror::Error;

use crate::domain::{config::SettingKey, pty::PtyError, value::ProcessName};

/// Separates names listed in an error message.
const NAME_SEPARATOR: &str = ", ";
/// Stands in for an empty list of names in an error message.
const NO_NAMES: &str = "none";

/// Errors from muster's stored state: projects, stacks, settings and
/// coordination.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// A file could not be written to disk.
    #[error("could not write {path}: {source}")]
    Write {
        /// Path that failed to write.
        path: PathBuf,
        /// Underlying I/O error.
        source: io::Error,
    },
    /// No state directory could be located on this platform.
    #[error("no state directory is available")]
    NoStateDir,
    /// A coordination id could not be generated (its UUID text was empty, which
    /// cannot happen in practice).
    #[error("could not generate a coordination id: {0}")]
    CoordinationId(String),
    /// The database could not be opened, read, or written. Carries a message
    /// rather than the driver's error type so no database library leaks into
    /// the domain.
    #[error("database error: {0}")]
    Database(String),
    /// A stored row holds a value the domain refuses: it was edited by hand
    /// or written by a different muster.
    #[error("stored {what} is invalid: {reason}")]
    CorruptRow {
        /// What the refused value is.
        what: String,
        /// Why it was refused.
        reason: String,
    },
    /// A value is too large for the database to store.
    #[error("{what} is too large to store")]
    TooLarge {
        /// What the value is.
        what: String,
    },
    /// The database was written by a newer muster.
    #[error(
        "the database is at schema version {found}, newer than the {supported} this muster supports; upgrade muster"
    )]
    UnsupportedSchema {
        /// The schema version the database records.
        found: u32,
        /// The newest schema version this binary knows.
        supported: u32,
    },
    /// No project is registered at a folder.
    #[error("no muster project at {}; `muster add` registers one", root.display())]
    UnknownProject {
        /// The folder that is not a project.
        root: PathBuf,
    },
    /// No process in the project has a name.
    #[error(
        "no process named '{name}' in this project; processes: {}",
        list(valid)
    )]
    UnknownProcess {
        /// The name asked for.
        name: ProcessName,
        /// The names the project defines.
        valid: Vec<ProcessName>,
    },
    /// A process of that name, by NFC key, already exists in the project.
    #[error("a process named '{name}' already exists in this project")]
    DuplicateProcessName {
        /// The name refused.
        name: ProcessName,
    },
    /// A strict import names processes the project already defines.
    #[error(
        "the project already defines {}; import with --merge or --replace",
        list(names)
    )]
    ImportConflict {
        /// Every imported name already defined.
        names: Vec<ProcessName>,
    },
    /// A setting's value is not one it accepts.
    #[error("invalid {key}: {reason}")]
    InvalidSetting {
        /// The setting refused.
        key: SettingKey,
        /// Why, naming the accepted form.
        reason: String,
    },
    /// The default size is one no PTY can take.
    #[error("invalid {}: {source}", SettingKey::DefaultSize)]
    InvalidDefaultSize {
        /// Why the size was refused.
        source: PtyError,
    },
    /// A project folder is missing or not a directory.
    #[error("{} is not a directory", path.display())]
    RootNotADirectory {
        /// The path refused.
        path: PathBuf,
    },
    /// A project folder's path is not UTF-8, so the database cannot store it.
    #[error("{} is not valid UTF-8 and cannot be a project folder", path.display())]
    RootNotUtf8 {
        /// The path refused.
        path: PathBuf,
    },
}

/// `names` joined for an error message.
fn list(names: &[ProcessName]) -> String {
    if names.is_empty() {
        return NO_NAMES.to_string();
    }
    names
        .iter()
        .map(|name| format!("'{name}'"))
        .collect::<Vec<_>>()
        .join(NAME_SEPARATOR)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(raw: &[&str]) -> Vec<ProcessName> {
        raw.iter()
            .map(|name| ProcessName::try_new(*name).unwrap())
            .collect()
    }

    #[test]
    fn unknown_process_lists_the_valid_names() {
        let error = ConfigError::UnknownProcess {
            name: ProcessName::try_new("wbe").unwrap(),
            valid: names(&["web", "api"]),
        };

        assert_eq!(
            error.to_string(),
            "no process named 'wbe' in this project; processes: 'web', 'api'"
        );
    }

    #[test]
    fn an_empty_project_lists_none() {
        let error = ConfigError::UnknownProcess {
            name: ProcessName::try_new("web").unwrap(),
            valid: Vec::new(),
        };

        assert!(error.to_string().ends_with("processes: none"));
    }
}
