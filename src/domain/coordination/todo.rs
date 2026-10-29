use std::{collections::HashMap, time::Duration};

use getset::Getters;
use nutype::nutype;
use serde::{Deserialize, Serialize};
use strum::{Display, EnumString};
use typed_builder::TypedBuilder;

use crate::domain::coordination::{Author, TodoChange, TodoUpdate};

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
    /// The agent working it and until when, only while it is in progress.
    claim: Option<TodoClaim>,
    /// Who last wrote the todo, stamped by the store.
    author: Author,
    /// Unix epoch seconds of the last write, stamped by the store.
    updated_at: u64,
}

impl Todo {
    /// Who holds this todo at `now` (Unix seconds), if anyone.
    #[must_use]
    pub fn claim_state(&self, now: u64) -> ClaimState<'_> {
        match &self.claim {
            None => ClaimState::Unclaimed,
            Some(claim) if claim.is_live(now) => ClaimState::Held {
                owner: claim.owner(),
                expires_in: claim.expires_in(now),
            },
            Some(claim) => ClaimState::Stale {
                owner: claim.owner(),
            },
        }
    }

    /// The status agents read at `now`: a todo whose lease ran out reads as
    /// pending, so abandoned work is picked up again.
    #[must_use]
    pub fn effective_status(&self, now: u64) -> TodoStatus {
        match self.claim_state(now) {
            ClaimState::Stale { .. } => TodoStatus::Pending,
            ClaimState::Unclaimed | ClaimState::Held { .. } => self.status,
        }
    }

    /// Whether an agent can claim this todo at `now` without naming it: it
    /// reads as pending and every dependency `is_done` (a dependency that no
    /// longer exists does not block).
    #[must_use]
    pub fn is_claimable(&self, now: u64, is_done: impl Fn(&TodoId) -> Option<bool>) -> bool {
        self.effective_status(now) == TodoStatus::Pending
            && self.deps.iter().all(|dep| is_done(dep).unwrap_or(true))
    }

    /// This todo with `change` applied by `me` at `now`, or who holds it when
    /// another agent's live lease guards it and `change` does not force.
    /// Any status but in-progress drops the claim.
    #[must_use]
    pub fn changed(&self, me: &Author, change: &TodoChange, now: u64) -> TodoUpdate {
        if let ClaimState::Held { owner, expires_in } = self.claim_state(now)
            && owner != me
            && !change.force()
        {
            return TodoUpdate::HeldBy {
                owner: owner.clone(),
                expires_in,
            };
        }
        let status = change.status().unwrap_or(self.status);
        TodoUpdate::Updated(
            Todo::builder()
                .id(self.id.clone())
                .title(change.title().clone().unwrap_or_else(|| self.title.clone()))
                .status(status)
                .deps(self.deps.clone())
                .claim(
                    self.claim
                        .clone()
                        .filter(|_| status == TodoStatus::InProgress),
                )
                .author(me.clone())
                .updated_at(now)
                .build(),
        )
    }
}

/// An agent's hold on an in-progress todo: who works it, and the Unix second
/// its lease runs out.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct TodoClaim {
    /// The agent working the todo.
    owner: Author,
    /// Unix epoch seconds after which the lease has run out.
    claimed_until: u64,
}

impl TodoClaim {
    /// Whether the lease still holds at `now`.
    #[must_use]
    pub fn is_live(&self, now: u64) -> bool {
        self.claimed_until >= now
    }

    /// How long the lease has left at `now`; zero once it has run out.
    #[must_use]
    pub fn expires_in(&self, now: u64) -> Duration {
        Duration::from_secs(self.claimed_until.saturating_sub(now))
    }
}

/// Who holds a todo at a moment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClaimState<'a> {
    /// Nobody claimed it.
    Unclaimed,
    /// An agent holds it under a live lease.
    Held {
        /// The agent holding it.
        owner: &'a Author,
        /// How long the lease has left.
        expires_in: Duration,
    },
    /// An agent claimed it and let the lease run out.
    Stale {
        /// The agent whose lease ran out.
        owner: &'a Author,
    },
}

/// The todos of one project an agent can claim at `now`, oldest first: the
/// ones [`Todo::is_claimable`] accepts, reading dependencies from `todos`.
pub fn claimable(todos: &[Todo], now: u64) -> impl Iterator<Item = &Todo> {
    let done: HashMap<&TodoId, bool> = todos
        .iter()
        .map(|todo| (todo.id(), todo.status == TodoStatus::Done))
        .collect();
    todos
        .iter()
        .filter(move |todo| todo.is_claimable(now, |id| done.get(id).copied()))
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

    /// A todo with `status`, `deps` and `claim`.
    fn todo(status: TodoStatus, deps: Vec<TodoId>, claim: Option<(&str, u64)>) -> Todo {
        Todo::builder()
            .id(TodoId::generate().unwrap())
            .title(TodoTitle::try_new("task").unwrap())
            .status(status)
            .deps(deps)
            .claim(claim.map(|(owner, until)| {
                TodoClaim::builder()
                    .owner(Author::agent(owner))
                    .claimed_until(until)
                    .build()
            }))
            .author(Author::human())
            .updated_at(0)
            .build()
    }

    fn change(status: Option<TodoStatus>, force: bool) -> TodoChange {
        TodoChange::builder()
            .title(None)
            .status(status)
            .force(force)
            .build()
    }

    /// A claim is held through its last second, then reads as stale and the
    /// todo as pending.
    #[test]
    fn a_lease_runs_out_after_its_last_second() {
        let claimed = todo(TodoStatus::InProgress, vec![], Some(("claude", 100)));
        assert_eq!(claimed.claim_state(40), ClaimState::Held {
            owner: &Author::agent("claude"),
            expires_in: Duration::from_secs(60),
        });
        assert_eq!(claimed.effective_status(100), TodoStatus::InProgress);
        assert_eq!(claimed.claim_state(101), ClaimState::Stale {
            owner: &Author::agent("claude"),
        });
        assert_eq!(claimed.effective_status(101), TodoStatus::Pending);
    }

    /// Only pending or stale todos whose dependencies are done, or gone, are
    /// claimable.
    #[test]
    fn claimable_needs_pending_and_done_deps() {
        let done = todo(TodoStatus::Done, vec![], None);
        let open = todo(TodoStatus::Pending, vec![], None);
        let ready = todo(TodoStatus::Pending, vec![done.id().clone()], None);
        let blocked = todo(TodoStatus::Pending, vec![open.id().clone()], None);
        let orphan = todo(TodoStatus::Pending, vec![TodoId::generate().unwrap()], None);
        let held = todo(TodoStatus::InProgress, vec![], Some(("claude", 100)));
        let stale = todo(TodoStatus::InProgress, vec![], Some(("claude", 10)));
        let unowned = todo(TodoStatus::InProgress, vec![], None);
        let all = vec![
            done,
            open.clone(),
            ready.clone(),
            blocked,
            orphan.clone(),
            held,
            stale.clone(),
            unowned,
        ];

        let ids: Vec<_> = claimable(&all, 50).map(Todo::id).collect();

        assert_eq!(ids, vec![open.id(), ready.id(), orphan.id(), stale.id()]);
    }

    /// Another agent's live lease refuses a change unless it forces; the
    /// owner, or anyone once the lease ran out, may change it.
    #[test]
    fn a_live_lease_guards_the_todo() {
        let claimed = todo(TodoStatus::InProgress, vec![], Some(("claude", 100)));
        let done = change(Some(TodoStatus::Done), false);

        assert_eq!(
            claimed.changed(&Author::agent("codex"), &done, 40),
            TodoUpdate::HeldBy {
                owner: Author::agent("claude"),
                expires_in: Duration::from_secs(60),
            }
        );
        for (me, now, force) in [
            ("codex", 40, true),
            ("claude", 40, false),
            ("codex", 101, false),
        ] {
            let TodoUpdate::Updated(updated) = claimed.changed(
                &Author::agent(me),
                &change(Some(TodoStatus::Done), force),
                now,
            ) else {
                panic!("{me} at {now} should change it");
            };
            assert_eq!(*updated.status(), TodoStatus::Done);
            assert_eq!(updated.claim(), &None, "done drops the claim");
            assert_eq!(updated.author(), &Author::agent(me));
            assert_eq!(*updated.updated_at(), now);
        }
    }

    /// Releasing hands the todo back as pending and drops the claim; a title
    /// change keeps the claim of an in-progress todo.
    #[test]
    fn release_drops_and_retitle_keeps_the_claim() {
        let claimed = todo(TodoStatus::InProgress, vec![], Some(("claude", 100)));
        let me = Author::agent("claude");

        let TodoUpdate::Updated(released) =
            claimed.changed(&me, &change(Some(TodoStatus::Pending), false), 40)
        else {
            panic!("the owner releases");
        };
        assert_eq!(*released.status(), TodoStatus::Pending);
        assert_eq!(released.claim(), &None);

        let retitle = TodoChange::builder()
            .title(Some(TodoTitle::try_new("renamed").unwrap()))
            .status(None)
            .force(false)
            .build();
        let TodoUpdate::Updated(renamed) = claimed.changed(&me, &retitle, 40) else {
            panic!("the owner renames");
        };
        assert_eq!(renamed.title().as_ref(), "renamed");
        assert_eq!(renamed.claim(), claimed.claim());
    }
}
