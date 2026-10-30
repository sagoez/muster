use getset::Getters;
use serde::Serialize;
use typed_builder::TypedBuilder;

use super::stack_process_view::StackProcessView;
use crate::adapter::client::{LockView, NoticeView, ProjectIdentityView};

/// The whole picture of one project's stack, as `stack_status` answers.
#[derive(Debug, Clone, PartialEq, Serialize, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct StackStatusView {
    /// The project.
    project: ProjectIdentityView,
    /// The label this connection writes as.
    you: String,
    /// Every process, in stack order.
    processes: Vec<StackProcessView>,
    /// The project's recent warnings and errors, oldest first.
    notices: Vec<NoticeView>,
    /// The locks held in the project.
    locks: Vec<LockView>,
    /// Pass as since to stack_events to read only later events.
    #[serde(skip_serializing_if = "Option::is_none")]
    events_cursor: Option<String>,
}
