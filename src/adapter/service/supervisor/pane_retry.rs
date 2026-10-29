use std::time::Duration;

use getset::CopyGetters;

use super::retry_id::RetryId;
use crate::domain::value::Pid;

/// One pane's retries of adopting the holder of a child left running
/// unsupervised, owned and built by [`super::adoption_retries::AdoptionRetries`].
#[derive(Debug, Clone, Copy, CopyGetters)]
pub(super) struct PaneRetry {
    /// The holder whose failed adoption was already told, so a retry that
    /// fails the same way is not told again.
    told: Option<Pid>,
    /// Wait before the next timed retry.
    delay: Duration,
    /// The current try; a result or timer of any other is stale.
    #[getset(get_copy = "pub(super)")]
    current: RetryId,
}

impl PaneRetry {
    /// A pane's first retries, whose first timed retry waits `delay`.
    pub(super) fn new(delay: Duration) -> Self {
        Self {
            told: None,
            delay,
            current: RetryId::default(),
        }
    }

    /// Whether the failure of `holder` is news, recording that it was told.
    pub(super) fn tell(&mut self, holder: Pid) -> bool {
        let news = self.told != Some(holder);
        self.told = Some(holder);
        news
    }

    /// Makes `id` the current try.
    pub(super) fn begin(&mut self, id: RetryId) {
        self.current = id;
    }

    /// Makes `id` the current try and returns how long to wait before it,
    /// doubling the wait of the next one up to `cap`.
    pub(super) fn schedule(&mut self, id: RetryId, cap: Duration) -> Duration {
        let delay = self.delay;
        self.delay = self.delay.saturating_mul(2).min(cap);
        self.current = id;
        delay
    }
}
