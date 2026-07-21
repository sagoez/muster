use std::path::PathBuf;

use thiserror::Error;

use crate::domain::{process::ProcessKind, value::ProcessName};

/// Errors from loading or parsing the workspace configuration.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// The configuration file could not be read from disk.
    #[error("could not read config file {path}: {source}")]
    Read {
        /// Path that failed to load.
        path: PathBuf,
        /// Underlying I/O error.
        source: std::io::Error,
    },
    /// The configuration file could not be written to disk.
    #[error("could not write config file {path}: {source}")]
    Write {
        /// Path that failed to write.
        path: PathBuf,
        /// Underlying I/O error.
        source: std::io::Error,
    },
    /// No writable config directory could be located on this platform.
    #[error("no config directory is available")]
    NoConfigDir,
    /// The configuration was not valid YAML or violated the schema.
    #[error("could not parse config: {0}")]
    Parse(#[from] serde_yaml_ng::Error),
    /// A non-command process configured command-only graceful shutdown.
    #[error("{kind} process '{name}' configures stop, which is only valid for commands")]
    InvalidStopPolicy {
        /// Section containing the invalid process.
        kind: ProcessKind,
        /// Name of the invalid process.
        name: ProcessName,
    },
}
