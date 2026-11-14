use clap::Subcommand;

/// Lock actions under `muster lock`. Absent, held locks are listed.
#[derive(Subcommand)]
pub enum LockCommand {
    /// Release a lock whoever holds it, to clear one an agent left behind.
    Release {
        /// Lock name.
        name: String,
    },
}
