use clap::Subcommand;

/// Scratchpad actions under `muster note`. Absent, notes are listed.
#[derive(Subcommand)]
pub enum NoteCommand {
    /// Print a note's body by key.
    Get {
        /// Note key.
        key: String,
    },
    /// Create or replace a note.
    Set {
        /// Note key.
        key: String,
        /// Markdown body.
        body: String,
    },
    /// Delete a note by key.
    Rm {
        /// Note key.
        key: String,
    },
}
