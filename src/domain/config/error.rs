use std::{error::Error as StdError, path::PathBuf};

use thiserror::Error;

use crate::domain::{
    config::{OverCap, PROCESSES_MAX, RepeatedPort, SettingKey, TooManyPorts},
    coordination::LOCKS_PER_SESSION_MAX,
    pty::PtyError,
    value::{ProcessName, ProjectName, refused_name},
};

/// Separates names listed in an error message.
const NAME_SEPARATOR: &str = ", ";
/// Stands in for an empty list of names in an error message.
const NO_NAMES: &str = "none";

/// Errors from muster's stored state: projects, stacks, settings and
/// coordination.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// A coordination id could not be generated (its UUID text was empty, which
    /// cannot happen in practice).
    #[error("could not generate a coordination id: {0}")]
    CoordinationId(String),
    /// The store behind a port failed. Opaque, so no storage library or path
    /// detail leaks into the domain.
    #[error(transparent)]
    Storage(Box<dyn StdError + Send + Sync + 'static>),
    /// The store stayed busy or locked by another writer past its own wait:
    /// the same write may land if tried again later. Opaque like
    /// [`ConfigError::Storage`].
    #[error(transparent)]
    Busy(Box<dyn StdError + Send + Sync + 'static>),
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
    /// A write would leave a process with a value past a cap.
    #[error("'{}': {over}", refused_name(name.as_ref()))]
    OverCap {
        /// The process written.
        name: ProcessName,
        /// The cap it would run past.
        over: OverCap,
    },
    /// A process would declare more ports than it may.
    #[error(transparent)]
    TooManyPorts(#[from] TooManyPorts),
    /// A list of ports names one port more than once.
    #[error(transparent)]
    RepeatedPort(#[from] RepeatedPort),
    /// A write would leave the project with more processes than it may have.
    #[error(
        "this would leave the project with {found} processes, more than the {PROCESSES_MAX} it may have; remove one first"
    )]
    TooManyProcesses {
        /// How many processes the project would hold.
        found: usize,
    },
    /// A connection would hold more live locks than it may.
    #[error(
        "this connection already holds {LOCKS_PER_SESSION_MAX} locks, the most it may; release one first"
    )]
    TooManyLocks,
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
    /// A folder would become a project holding a registered one, which
    /// another write registered first.
    #[error(
        "{} holds the project '{project}', so it cannot be a muster project",
        dir.display()
    )]
    HoldsProject {
        /// The folder refused.
        dir: PathBuf,
        /// The registered project inside it.
        project: ProjectName,
    },
}

impl ConfigError {
    /// Whether the store was only busy, so the same write may land later.
    #[must_use]
    pub const fn is_busy(&self) -> bool {
        matches!(self, Self::Busy(_))
    }
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

    /// The process names `raw`.
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
