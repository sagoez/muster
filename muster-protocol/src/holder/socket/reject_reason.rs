use serde::{Deserialize, Serialize};

/// Why the holder refused a client message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RejectReason {
    /// The client sent a message before introducing itself.
    HelloRequired,
    /// The client sent a second hello; its role is fixed by the first.
    AlreadyIntroduced,
    /// A read-only client tried to write or resize.
    ReadOnly,
    /// A non-supervisor tried to control the child or collect its exit.
    SupervisorOnly,
    /// An exit was collected before the child exited.
    NotExited,
    /// The child refused the operation, typically because it has exited.
    ChildUnavailable,
    /// The child is not reading its input fast enough; the input was dropped.
    InputBacklogged,
}
