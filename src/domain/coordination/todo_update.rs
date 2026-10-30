use std::time::Duration;

use crate::domain::coordination::{Author, Todo};

/// What a todo change came to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TodoUpdate {
    /// The todo as stored after the change.
    Updated(Todo),
    /// Another agent holds the todo under a live lease and the change did not
    /// force.
    HeldBy {
        /// The agent holding it.
        owner: Author,
        /// How long its lease has left.
        expires_in: Duration,
    },
    /// No todo has the id.
    Missing,
}
