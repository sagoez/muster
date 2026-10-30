use getset::{CopyGetters, Getters};
use typed_builder::TypedBuilder;

use crate::adapter::service::Notice;

/// A project's notices read without starting the service: the stored ones,
/// and a running service's own when one runs.
#[derive(Debug, Clone, PartialEq, Getters, CopyGetters, TypedBuilder)]
pub struct NoticeLog {
    /// The notices, oldest first.
    #[getset(get = "pub")]
    notices: Vec<Notice>,
    /// Whether a service ran, so its live notices are among them.
    #[getset(get_copy = "pub")]
    service_running: bool,
}
