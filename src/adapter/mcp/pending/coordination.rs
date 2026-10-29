use std::time::Duration;

use getset::{CopyGetters, Getters};
use nutype::nutype;
use typed_builder::TypedBuilder;

use crate::{
    adapter::mcp::tool_error::ToolError,
    domain::{
        coordination::{Author, Todo, TodoId, TodoStatus, TodoTitle},
        port::CoordinationStore,
        value::ProjectRoot,
    },
};

/// Longest lock name.
const LOCK_NAME_MAX_CHARS: usize = 64;
/// The feature these stand-ins wait for.
const CLAIMS_AND_LOCKS: &str = "todo claims and locks (package C)";

/// Whether `name` uses only the characters a lock name allows.
fn is_lock_name(name: &str) -> bool {
    name.chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'))
}

/// A lock's name. Stand-in for package C's `LockName`.
#[nutype(
    sanitize(trim),
    validate(not_empty, len_char_max = LOCK_NAME_MAX_CHARS, predicate = |name: &str| is_lock_name(name)),
    derive(Debug, Clone, PartialEq, Eq, AsRef, Display)
)]
pub struct LockName(String);

/// A held lock. Stand-in for package C's `Lock`.
#[derive(Debug, Clone, PartialEq, Eq, Getters, CopyGetters, TypedBuilder)]
pub struct Lock {
    /// Its name.
    #[getset(get = "pub")]
    name: LockName,
    /// Who holds it.
    #[getset(get = "pub")]
    owner: Author,
    /// Why, as its holder said.
    #[getset(get = "pub")]
    note: Option<String>,
    /// Unix seconds when it lapses.
    #[getset(get_copy = "pub")]
    expires_at: u64,
}

/// What an attempt at a lock came to. Stand-in for package C's
/// `LockAttempt`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LockAttempt {
    /// The caller holds it now.
    Acquired {
        /// Until it lapses.
        expires_in: Duration,
    },
    /// Someone else holds it.
    HeldBy {
        /// Who.
        owner: Author,
        /// Why, as they said.
        note: Option<String>,
        /// Until it lapses.
        expires_in: Duration,
    },
}

/// What a release came to. Stand-in for package C's `LockRelease`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LockRelease {
    /// The caller held it and it is free now.
    Released,
    /// Nobody held it.
    NotHeld,
    /// Someone else holds it.
    HeldBy {
        /// Who.
        owner: Author,
    },
}

/// What a claim came to. Stand-in for package C's `Claim`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Claim {
    /// The caller holds the todo now.
    Claimed(Todo),
    /// Another agent holds it under a live lease.
    HeldBy {
        /// Who.
        owner: Author,
        /// Until the lease lapses.
        expires_in: Duration,
    },
    /// No todo was claimable.
    NothingClaimable,
}

/// A change to one todo. Stand-in for package C's `TodoChange`.
#[derive(Debug, Clone, PartialEq, Eq, Getters, CopyGetters, TypedBuilder)]
pub struct TodoChange {
    /// Its new status.
    #[getset(get_copy = "pub")]
    status: Option<TodoStatus>,
    /// Its new title.
    #[getset(get = "pub")]
    title: Option<TodoTitle>,
    /// Hand it back as pending, clearing its owner.
    #[getset(get_copy = "pub")]
    release: bool,
    /// Change it even when another agent holds it.
    #[getset(get_copy = "pub")]
    force: bool,
}

/// What a todo change came to. Stand-in for package C's `TodoUpdate`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TodoUpdate {
    /// The todo as it is now.
    Updated(Todo),
    /// Another agent holds it under a live lease.
    HeldBy {
        /// Who.
        owner: Author,
        /// Until the lease lapses.
        expires_in: Duration,
    },
    /// No todo has the id.
    Missing,
}

/// A todo's claim: who holds it and until when. Stand-in for package C's
/// `owner` and `claimed_until` on [`Todo`].
#[derive(Debug, Clone, PartialEq, Eq, Getters, CopyGetters, TypedBuilder)]
pub struct TodoLease {
    /// Who holds it.
    #[getset(get = "pub")]
    owner: Author,
    /// Unix seconds when the claim lapses.
    #[getset(get_copy = "pub")]
    claimed_until: u64,
}

/// The claim on `todo`, if any. Stand-in for package C's `Todo::owner`.
pub fn lease_of(_todo: &Todo) -> Option<TodoLease> {
    None
}

/// Claims the todo `id`, or the oldest claimable one. Stand-in for package
/// C's `CoordinationStore::claim_todo`.
///
/// # Errors
/// Always [`ToolError::Pending`] until package C lands.
pub fn claim_todo(
    _store: &dyn CoordinationStore,
    _project: &ProjectRoot,
    _me: &Author,
    _id: Option<&TodoId>,
    _lease: Duration,
) -> Result<Claim, ToolError> {
    Err(ToolError::Pending(CLAIMS_AND_LOCKS))
}

/// Applies `change` to the todo `id`. Stand-in for package C's
/// `CoordinationStore::update_todo`.
///
/// # Errors
/// Always [`ToolError::Pending`] until package C lands.
pub fn update_todo(
    _store: &dyn CoordinationStore,
    _project: &ProjectRoot,
    _me: &Author,
    _id: &TodoId,
    _change: &TodoChange,
) -> Result<TodoUpdate, ToolError> {
    Err(ToolError::Pending(CLAIMS_AND_LOCKS))
}

/// Takes or renews the lock `name`. Stand-in for package C's
/// `CoordinationStore::acquire_lock`.
///
/// # Errors
/// Always [`ToolError::Pending`] until package C lands.
pub fn acquire_lock(
    _store: &dyn CoordinationStore,
    _project: &ProjectRoot,
    _me: &Author,
    _name: &LockName,
    _ttl: Duration,
    _note: Option<&str>,
) -> Result<LockAttempt, ToolError> {
    Err(ToolError::Pending(CLAIMS_AND_LOCKS))
}

/// Releases the lock `name` when the caller holds it. Stand-in for package
/// C's `CoordinationStore::release_lock`.
///
/// # Errors
/// Always [`ToolError::Pending`] until package C lands.
pub fn release_lock(
    _store: &dyn CoordinationStore,
    _project: &ProjectRoot,
    _me: &Author,
    _name: &LockName,
) -> Result<LockRelease, ToolError> {
    Err(ToolError::Pending(CLAIMS_AND_LOCKS))
}

/// Releases every lock `me` holds, in any project. Stand-in for package C's
/// `CoordinationStore::release_locks_of`.
///
/// # Errors
/// Always [`ToolError::Pending`] until package C lands.
pub fn release_locks_of(_store: &dyn CoordinationStore, _me: &Author) -> Result<usize, ToolError> {
    Err(ToolError::Pending(CLAIMS_AND_LOCKS))
}

/// The live locks of `project`. Stand-in for package C's
/// `CoordinationStore::locks`; it holds none until package C lands.
///
/// # Errors
/// Never fails until package C lands.
pub fn locks(
    _store: &dyn CoordinationStore,
    _project: &ProjectRoot,
) -> Result<Vec<Lock>, ToolError> {
    Ok(Vec::new())
}
