use getset::{CopyGetters, Getters};
use muster_domain::coordination::LockAttempt;
use serde::Serialize;
use typed_builder::TypedBuilder;

/// What `lock_acquire` answers: whether it took the lock, how long it holds,
/// and who holds it when another agent does.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Getters, CopyGetters, TypedBuilder)]
pub struct AttemptView {
    /// Whether this call took the lock.
    #[getset(get_copy = "pub")]
    acquired: bool,
    /// Who holds the lock, when another agent does.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[builder(default)]
    #[getset(get = "pub")]
    owner: Option<String>,
    /// The holder's note, when another agent holds the lock: present, and
    /// null when the holder left none.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[builder(default)]
    #[getset(get = "pub")]
    note: Option<Option<String>>,
    /// Seconds until the lock runs out, when it is held.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[builder(default)]
    #[getset(get_copy = "pub")]
    expires_in_s: Option<u64>,
}

impl From<&LockAttempt> for AttemptView {
    fn from(attempt: &LockAttempt) -> Self {
        match attempt {
            LockAttempt::Acquired { expires_in } => Self::builder()
                .acquired(true)
                .expires_in_s(Some(expires_in.as_secs()))
                .build(),
            LockAttempt::HeldBy {
                owner,
                note,
                expires_in,
            } => Self::builder()
                .acquired(false)
                .owner(Some(owner.to_string()))
                .note(Some(note.as_ref().map(ToString::to_string)))
                .expires_in_s(Some(expires_in.as_secs()))
                .build(),
            LockAttempt::SessionEnded => Self::builder().acquired(false).build(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use muster_domain::coordination::{Author, LockNote};
    use serde_json::{Value, json};

    use super::*;

    /// How long the lock in these tests has left.
    const EXPIRES_IN: Duration = Duration::from_secs(60);

    /// The JSON an agent receives for `attempt`.
    fn answered(attempt: &LockAttempt) -> Value {
        serde_json::to_value(AttemptView::from(attempt)).unwrap()
    }

    /// A lock taken says how long it holds; a lock held elsewhere names its
    /// holder and its note, null when the holder left none; a lock refused
    /// because the session ended says only that.
    #[test]
    fn each_attempt_answers_its_fields() {
        assert_eq!(
            answered(&LockAttempt::Acquired {
                expires_in: EXPIRES_IN
            }),
            json!({ "acquired": true, "expires_in_s": 60 })
        );
        assert_eq!(
            answered(&LockAttempt::HeldBy {
                owner: Author::agent("codex"),
                note: Some(LockNote::try_new("migrating").unwrap()),
                expires_in: EXPIRES_IN,
            }),
            json!({ "acquired": false, "owner": "codex", "note": "migrating", "expires_in_s": 60 })
        );
        assert_eq!(
            answered(&LockAttempt::HeldBy {
                owner: Author::agent("codex"),
                note: None,
                expires_in: EXPIRES_IN,
            }),
            json!({ "acquired": false, "owner": "codex", "note": null, "expires_in_s": 60 })
        );
        assert_eq!(
            answered(&LockAttempt::SessionEnded),
            json!({ "acquired": false })
        );
    }
}
