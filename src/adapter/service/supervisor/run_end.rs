use std::time::SystemTime;

use getset::CopyGetters;
use typed_builder::TypedBuilder;

use super::SpawnGeneration;
use crate::adapter::service::ExitReason;

/// How one pane's most recent run ended, kept beside its exit outcome so
/// `ps` can say why and a wait on that run can say how it ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, CopyGetters, TypedBuilder)]
pub(crate) struct RunEnd {
    /// The run that ended.
    #[getset(get_copy = "pub(super)")]
    generation: SpawnGeneration,
    /// When it ended.
    #[getset(get_copy = "pub(super)")]
    at: SystemTime,
    /// Why it ended.
    #[getset(get_copy = "pub(super)")]
    reason: ExitReason,
}
