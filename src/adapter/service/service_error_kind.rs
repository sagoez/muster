use serde::{Deserialize, Serialize};

/// Why the service refused a request, for a client to act on without parsing
/// the message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ServiceErrorKind {
    /// The project has no process of that name; the valid names come with it.
    UnknownProcess,
    /// No project config exists at that path.
    UnknownProject,
    /// The project's config exists but cannot be loaded.
    InvalidConfig,
    /// The process has no running child to reach.
    NotRunning,
    /// The project refused the request, such as an agent session it could not
    /// open.
    Refused,
    /// The request could not be read, as one from a client of another version
    /// would be.
    InvalidRequest,
    /// The service is shutting down and takes no more requests.
    Busy,
    /// The service failed for a reason of its own, such as a store it cannot
    /// read; nothing about the request was wrong.
    Internal,
}
