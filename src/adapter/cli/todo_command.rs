use clap::Subcommand;

use crate::domain::coordination::TodoStatus;

/// Todo actions under `muster todo`. Absent, todos are listed.
#[derive(Subcommand)]
pub enum TodoCommand {
    /// Add a new pending todo.
    Add {
        /// What the task is.
        title: String,
        /// Id of a todo this one depends on (repeatable).
        #[arg(long = "dep")]
        deps: Vec<String>,
    },
    /// Set a todo's status: pending, in-progress, or done.
    Status {
        /// Todo id.
        id: String,
        /// New status.
        status: TodoStatus,
        /// Change it even though an agent holds it.
        #[arg(long)]
        force: bool,
    },
    /// Delete a todo by id.
    Rm {
        /// Todo id.
        id: String,
    },
}
