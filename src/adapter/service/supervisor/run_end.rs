use std::time::SystemTime;

use super::SpawnGeneration;
use crate::adapter::service::ExitReason;

/// How one pane's most recent run ended, kept beside its exit outcome so
/// `ps` can say why and a wait on that run can say how it ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RunEnd {
    /// The run that ended.
    pub(super) generation: SpawnGeneration,
    /// When it ended.
    pub(super) at: SystemTime,
    /// Why it ended.
    pub(super) reason: ExitReason,
}
