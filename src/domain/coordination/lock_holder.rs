use getset::Getters;
use typed_builder::TypedBuilder;

use crate::domain::coordination::{Author, SessionId};

/// Most live locks one connection holds at once, so its locks cannot flood
/// every agent's `stack_status`.
pub const LOCKS_PER_SESSION_MAX: usize = 32;

/// Who takes, renews and releases a lock: an agent through one connection.
/// Another connection under the same label is another holder.
#[derive(Clone, Debug, PartialEq, Eq, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct LockHolder {
    /// The agent, as the lock's owner is shown.
    author: Author,
    /// The connection it holds the lock through.
    session: SessionId,
}

#[cfg(test)]
impl LockHolder {
    /// The agent `label` through a session of its own.
    pub fn fresh(label: &str) -> Self {
        Self::builder()
            .author(Author::agent(label))
            .session(SessionId::generate())
            .build()
    }
}
