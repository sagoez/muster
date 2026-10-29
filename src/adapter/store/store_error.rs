use std::{io, path::PathBuf};

use thiserror::Error;

use crate::domain::config::ConfigError;

/// Failures of the SQLite store itself, which reach the domain as an opaque
/// [`ConfigError::Storage`] so no database or filesystem detail leaks into it.
#[derive(Debug, Error)]
pub enum StoreError {
    /// No state directory could be located on this platform.
    #[error("no state directory is available")]
    NoStateDir,
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
    Sqlite(#[from] rusqlite::Error),
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
    Encode(#[from] serde_json::Error),
}

impl From<StoreError> for ConfigError {
    /// Hands a store failure to the domain as an opaque storage failure.
    fn from(error: StoreError) -> Self {
        Self::Storage(Box::new(error))
    }
}

#[cfg(test)]
mod tests {
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
}
