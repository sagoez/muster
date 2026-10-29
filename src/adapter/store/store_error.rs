use std::{io, path::PathBuf};

use rusqlite::{Error as SqliteError, ErrorCode};
use serde_json::Error as JsonError;
use thiserror::Error;

use crate::{adapter::no_state_dir::NoStateDir, domain::config::ConfigError};

/// Failures of the SQLite store itself, which reach the domain as an opaque
/// [`ConfigError::Storage`] or [`ConfigError::Busy`] so no database or
/// filesystem detail leaks into it.
#[derive(Debug, Error)]
pub enum StoreError {
    /// No state directory could be located on this platform.
    #[error(transparent)]
    NoStateDir(#[from] NoStateDir),
    /// The state directory or the database file could not be prepared.
    #[error("could not write {}: {source}", path.display())]
    Prepare {
        /// The path that could not be prepared.
        path: PathBuf,
        /// Underlying I/O error.
        source: io::Error,
    },
    /// SQLite failed to open, read or write the database.
    #[error("database error: {0}")]
    Sqlite(#[from] SqliteError),
    /// A previous user of the shared connection panicked while holding it.
    #[error("database error: the database connection lock is poisoned")]
    Poisoned,
    /// The settings row the initial migration seeds is gone.
    #[error("database error: the settings row is missing")]
    MissingSettings,
    /// A lock row vanished between its upsert and its read back.
    #[error("database error: lock '{0}' vanished mid-acquire")]
    LockVanished(String),
    /// A path to store is not UTF-8, which a text column cannot hold.
    #[error("database error: {} is not valid UTF-8", .0.display())]
    NotUtf8(PathBuf),
    /// A value could not be encoded into the JSON its column holds.
    #[error("database error: {0}")]
    Encode(#[from] JsonError),
}

impl StoreError {
    /// Whether SQLite refused because another writer held the database.
    fn is_busy(&self) -> bool {
        match self {
            Self::Sqlite(error) => matches!(
                error.sqlite_error_code(),
                Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked)
            ),
            Self::NoStateDir(_)
            | Self::Prepare { .. }
            | Self::Poisoned
            | Self::MissingSettings
            | Self::LockVanished(_)
            | Self::NotUtf8(_)
            | Self::Encode(_) => false,
        }
    }
}

impl From<StoreError> for ConfigError {
    /// Hands a store failure to the domain opaque: as a busy store when
    /// another writer held it, else as a storage failure.
    fn from(error: StoreError) -> Self {
        if error.is_busy() {
            Self::Busy(Box::new(error))
        } else {
            Self::Storage(Box::new(error))
        }
    }
}

#[cfg(test)]
mod tests {
    use rusqlite::ffi::{Error as FfiError, SQLITE_BUSY, SQLITE_CONSTRAINT, SQLITE_LOCKED};

    use super::*;

    /// A store failure reaches the domain opaque, speaking with its own words.
    #[test]
    fn a_store_failure_reaches_the_domain_as_its_own_message() {
        let error = ConfigError::from(StoreError::MissingSettings);

        assert!(matches!(error, ConfigError::Storage(_)));
        assert_eq!(
            error.to_string(),
            "database error: the settings row is missing"
        );
    }

    /// A busy or locked database reaches the domain as busy, so a writer may
    /// retry it; any other SQLite failure does not.
    #[test]
    fn only_a_busy_or_locked_database_is_busy() {
        let failure = |code| {
            ConfigError::from(StoreError::Sqlite(SqliteError::SqliteFailure(
                FfiError::new(code),
                None,
            )))
        };

        assert!(failure(SQLITE_BUSY).is_busy());
        assert!(failure(SQLITE_LOCKED).is_busy());
        assert!(!failure(SQLITE_CONSTRAINT).is_busy());
        assert!(!ConfigError::from(StoreError::MissingSettings).is_busy());
    }
}
