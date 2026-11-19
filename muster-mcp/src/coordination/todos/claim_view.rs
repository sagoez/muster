use getset::{CopyGetters, Getters};
use muster_client::TodoView;
use muster_domain::{coordination::Claim, value::EpochSeconds};
use serde::Serialize;
use typed_builder::TypedBuilder;

/// What `todo_claim` answers: whether it claimed a todo, and the todo or
/// who holds it when it did not.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Getters, CopyGetters, TypedBuilder)]
pub struct ClaimView {
    /// Whether this call claimed the todo.
    #[getset(get_copy = "pub")]
    claimed: bool,
    /// The todo claimed, or the one that cannot be claimed yet.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[builder(default)]
    #[getset(get = "pub")]
    todo: Option<TodoView>,
    /// Who holds the todo, when another agent does.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[builder(default)]
    #[getset(get = "pub")]
    owner: Option<String>,
    /// Seconds until that agent's claim runs out.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[builder(default)]
    #[getset(get_copy = "pub")]
    expires_in_s: Option<u64>,
}

impl ClaimView {
    /// `claim` as read at `now`, Unix seconds.
    pub fn of(claim: &Claim, now: EpochSeconds) -> Self {
        match claim {
            Claim::Claimed(todo) => Self::builder()
                .claimed(true)
                .todo(Some(TodoView::at(todo, now)))
                .build(),
            Claim::HeldBy { owner, expires_in } => Self::builder()
                .claimed(false)
                .owner(Some(owner.to_string()))
                .expires_in_s(Some(expires_in.as_secs()))
                .build(),
            Claim::Unavailable(todo) => Self::builder()
                .claimed(false)
                .todo(Some(TodoView::at(todo, now)))
                .build(),
            Claim::NothingClaimable | Claim::Missing => Self::builder().claimed(false).build(),
        }
    }
}
