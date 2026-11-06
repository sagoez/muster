use std::path::PathBuf;

use clap::Subcommand;

/// Snapshot actions under `muster coordination`.
#[derive(Subcommand)]
pub enum CoordinationCommand {
    /// Write this project's scratchpads, todos, and key-values to a YAML file.
    Export {
        /// File to write.
        file: PathBuf,
    },
    /// Restore entries from a YAML file, preserving ids, authors, and timestamps.
    /// Entries merge; those absent from the file are left alone.
    Import {
        /// File to read.
        file: PathBuf,
    },
}
