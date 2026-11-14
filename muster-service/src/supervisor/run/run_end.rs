use std::time::SystemTime;

use getset::CopyGetters;
use muster_protocol::service::ExitReason;
use typed_builder::TypedBuilder;

use crate::supervisor::SpawnGeneration;

/// How one pane's most recent run ended, kept beside its exit outcome so
/// `ps` can say why and a wait on that run can say how it ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, CopyGetters, TypedBuilder)]
pub(crate) struct RunEnd {
    /// The run that ended.
    #[getset(get_copy = "pub(crate)")]
    generation: SpawnGeneration,
    /// When it ended.
    #[getset(get_copy = "pub(crate)")]
    at: SystemTime,
    /// Why it ended.
    #[getset(get_copy = "pub(crate)")]
    reason: ExitReason,
}
