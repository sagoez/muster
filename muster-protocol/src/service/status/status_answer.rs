use getset::{CopyGetters, Getters};
use serde::{Deserialize, Serialize};
use typed_builder::TypedBuilder;

use super::project_status::ProjectStatus;
use crate::service::events::event_seq::EventSeq;

/// The service's answer to `Ps`: each requested project's processes, and
/// the newest event of its log when it built them, so a client that also
/// reads the events knows which ones the answer already shows.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Getters, CopyGetters, TypedBuilder)]
#[serde(rename_all = "kebab-case")]
pub struct StatusAnswer {
    /// Each requested project's status, ordered by project folder.
    #[getset(get = "pub")]
    projects: Vec<ProjectStatus>,
    /// The newest event the statuses reflect: every event up to it happened
    /// before they were read, every later one after.
    #[getset(get_copy = "pub")]
    cursor: EventSeq,
}

impl StatusAnswer {
    /// The projects' statuses, giving up the cursor.
    #[must_use]
    pub fn into_projects(self) -> Vec<ProjectStatus> {
        self.projects
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::ServiceResponse;

    /// A status answer travels with its cursor as text and reads back the
    /// same.
    #[test]
    fn a_status_answer_carries_its_cursor_on_the_wire() {
        let answer = ServiceResponse::Status(
            StatusAnswer::builder()
                .projects(Vec::new())
                .cursor(EventSeq::builder().epoch(7).seq(42).build())
                .build(),
        );

        let wire = serde_json::to_value(&answer).unwrap();

        assert_eq!(
            wire,
            serde_json::json!({"status": {"projects": [], "cursor": "e:7:42"}})
        );
        assert_eq!(
            serde_json::from_value::<ServiceResponse>(wire).unwrap(),
            answer
        );
    }
}
