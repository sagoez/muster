use getset::{CopyGetters, Getters};
use serde::Serialize;
use typed_builder::TypedBuilder;

use super::pending::Lock;
use crate::domain::coordination::Author;

/// A held lock as `stack_status` lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Getters, CopyGetters, TypedBuilder)]
pub struct LockView {
    /// Its name.
    #[getset(get = "pub")]
    name: String,
    /// Who holds it.
    #[getset(get = "pub")]
    owner: Author,
    /// Why, as its holder said.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[getset(get = "pub")]
    note: Option<String>,
    /// Seconds until it lapses.
    #[getset(get_copy = "pub")]
    expires_in_s: u64,
}

impl LockView {
    /// `lock` as read at `now`, Unix seconds.
    pub fn at(lock: &Lock, now: u64) -> Self {
        Self {
            name: lock.name().to_string(),
            owner: lock.owner().clone(),
            note: lock.note().clone(),
            expires_in_s: lock.expires_at().saturating_sub(now),
        }
    }
}
