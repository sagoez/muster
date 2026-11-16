use clap::Subcommand;

/// Key-value actions under `muster kv`. Absent, entries are listed.
#[derive(Subcommand)]
pub enum KvCommand {
    /// Print a value by key.
    Get {
        /// Entry key.
        key: String,
    },
    /// Create or replace a value.
    ///
    /// Registers the folder as a project when no project covers it yet.
    Set {
        /// Entry key.
        key: String,
        /// Value to store.
        value: String,
    },
    /// Delete an entry by key.
    Rm {
        /// Entry key.
        key: String,
    },
}

impl KvCommand {
    /// Whether it writes a value, the one action that registers its folder.
    pub fn registers(&self) -> bool {
        matches!(self, Self::Set { .. })
    }
}
