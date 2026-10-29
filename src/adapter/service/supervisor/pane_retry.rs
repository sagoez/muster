use std::time::Duration;

use crate::domain::value::Pid;

/// One pane's retries of adopting the holder of a child left running
/// unsupervised, owned and built by [`super::adoption_retries::AdoptionRetries`].
#[derive(Debug, Clone, Copy)]
pub(super) struct PaneRetry {
    /// The holder whose failed adoption was already told, so a retry that
    /// fails the same way is not told again.
    pub(super) told: Option<Pid>,
    /// Wait before the next timed retry.
    pub(super) delay: Duration,
    /// The current try; a result or timer of any other is stale.
    pub(super) current: u64,
}
