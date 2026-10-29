use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use strum::IntoStaticStr;

use super::{
    batch_outcome::BatchOutcome, located::Located, notice::Notice, project_status::ProjectStatus,
    service_error_body::ServiceErrorBody, service_version::ServiceVersion, stack_event::EventPage,
    wait_outcome::WaitOutcome,
};

/// The service's answer to one [`ServiceRequest`](super::ServiceRequest), as
/// one JSON line. Its kind, as `&'static str`, names it in a message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, IntoStaticStr)]
#[serde(rename_all = "kebab-case")]
#[strum(serialize_all = "kebab-case")]
pub enum ServiceResponse {
    /// The service's own build.
    Hello {
        /// The service's muster version.
        version: ServiceVersion,
        /// The muster binary the service runs.
        binary: PathBuf,
    },
    /// The request was carried out, or handed to supervision; what supervision
    /// could not do lands in the notices.
    Ok,
    /// What `Up` or `Down` started or stopped, and what it left alone.
    Batch(BatchOutcome),
    /// The log a process's runs write to.
    Log(PathBuf),
    /// Each requested project's processes.
    Status(Vec<ProjectStatus>),
    /// Where a running process can be reached.
    Located(Located),
    /// A process's visible screen.
    Screen(String),
    /// Recent notices, oldest first.
    Notices(Vec<Notice>),
    /// How a wait ended.
    Waited(WaitOutcome),
    /// Lifecycle events after a cursor.
    Events(EventPage),
    /// The request was refused.
    Error(ServiceErrorBody),
}
