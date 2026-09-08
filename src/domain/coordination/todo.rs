use getset::Getters;
use nutype::nutype;
use serde::{Deserialize, Serialize};
use strum::{Display, EnumString};
use typed_builder::TypedBuilder;

use crate::domain::coordination::Author;

/// Stable identity of a todo within a project, generated when the todo is added.
#[nutype(
    sanitize(trim),
    validate(not_empty),
    derive(
        Debug,
        Clone,
        PartialEq,
        Eq,
        Hash,
        AsRef,
        Display,
        Serialize,
        Deserialize
    )
)]
pub struct TodoId(String);

impl TodoId {
    /// Creates a globally unique identity for a new todo.
    ///
    /// # Errors
    /// Returns the generated newtype's validation error if its invariant ever
    /// diverges from UUID's non-empty textual representation.
    pub fn generate() -> Result<Self, TodoIdError> {
        Self::try_new(uuid::Uuid::new_v4().to_string())
    }
}

/// A todo's human-readable title. Trimmed and non-empty, so a blank line can
/// never masquerade as a task.
#[nutype(
    sanitize(trim),
    validate(not_empty),
    derive(
        Debug,
        Clone,
        PartialEq,
        Eq,
        Hash,
        AsRef,
        Display,
        Serialize,
        Deserialize
    )
)]
pub struct TodoTitle(String);

/// Where a todo sits in its lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Display, EnumString, Serialize, Deserialize)]
#[strum(serialize_all = "kebab-case")]
#[serde(rename_all = "kebab-case")]
pub enum TodoStatus {
    /// Not started.
    Pending,
    /// Actively being worked.
    InProgress,
    /// Finished.
    Done,
}

/// A structured unit of shared work: a titled task with a lifecycle status and
/// optional dependencies on other todos, so agents and the human coordinate who
/// does what and in which order. The store stamps `updated_at`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct Todo {
    /// The todo's stable identity.
    id: TodoId,
    /// What the task is.
    title: TodoTitle,
    /// Where the task sits in its lifecycle.
    status: TodoStatus,
    /// Ids of todos this one depends on. Persisted as data; the graph is not
    /// validated here.
    deps: Vec<TodoId>,
    /// Who last wrote the todo, stamped by the store.
    author: Author,
    /// Unix epoch seconds of the last write, stamped by the store.
    updated_at: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Generated ids are unique and non-empty.
    #[test]
    fn generated_ids_are_unique() {
        let one = TodoId::generate().unwrap();
        let two = TodoId::generate().unwrap();
        assert_ne!(one, two);
        assert!(!one.as_ref().is_empty());
    }

    /// A blank title is rejected.
    #[test]
    fn a_blank_title_is_rejected() {
        assert!(TodoTitle::try_new("   ").is_err());
        assert_eq!(TodoTitle::try_new("  ship  ").unwrap().as_ref(), "ship");
    }

    /// Status renders and parses as kebab-case, so `in-progress` round-trips.
    #[test]
    fn status_renders_kebab_case() {
        assert_eq!(TodoStatus::InProgress.to_string(), "in-progress");
        assert_eq!(
            "in-progress".parse::<TodoStatus>().unwrap(),
            TodoStatus::InProgress
        );
    }
}
