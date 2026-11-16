use clap::Subcommand;
use muster_domain::coordination::TodoStatus;

/// Todo actions under `muster todo`. Absent, todos are listed.
#[derive(Subcommand)]
pub enum TodoCommand {
    /// Add a new pending todo.
    ///
    /// Registers the folder as a project when no project covers it yet.
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

impl TodoCommand {
    /// Whether it adds a todo, the one action that registers its folder.
    pub fn registers(&self) -> bool {
        matches!(self, Self::Add { .. })
    }
}
