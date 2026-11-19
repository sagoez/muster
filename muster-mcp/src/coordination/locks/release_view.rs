use getset::{CopyGetters, Getters};
use muster_domain::coordination::LockRelease;
use serde::Serialize;
use typed_builder::TypedBuilder;

/// What `lock_release` answers: whether it freed the lock, and who holds it
/// when another agent does.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Getters, CopyGetters, TypedBuilder)]
pub struct ReleaseView {
    /// Whether this call freed the lock.
    #[getset(get_copy = "pub")]
    released: bool,
    /// Who holds the lock, when another agent does.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[builder(default)]
    #[getset(get = "pub")]
    owner: Option<String>,
}

impl From<&LockRelease> for ReleaseView {
    fn from(release: &LockRelease) -> Self {
        match release {
            LockRelease::Released => Self::builder().released(true).build(),
            LockRelease::NotHeld => Self::builder().released(false).build(),
            LockRelease::HeldBy { owner } => Self::builder()
                .released(false)
                .owner(Some(owner.to_string()))
                .build(),
        }
    }
}

#[cfg(test)]
mod tests {
    use muster_domain::coordination::Author;
    use serde_json::{Value, json};

    use super::*;

    /// The JSON an agent receives for `release`.
    fn answered(release: &LockRelease) -> Value {
        serde_json::to_value(ReleaseView::from(release)).unwrap()
    }

    /// A release names the holder only when another agent holds the lock.
    #[test]
    fn each_release_answers_its_fields() {
        assert_eq!(
            answered(&LockRelease::Released),
            json!({ "released": true })
        );
        assert_eq!(
            answered(&LockRelease::NotHeld),
            json!({ "released": false })
        );
        assert_eq!(
            answered(&LockRelease::HeldBy {
                owner: Author::agent("codex")
            }),
            json!({ "released": false, "owner": "codex" })
        );
    }
}
