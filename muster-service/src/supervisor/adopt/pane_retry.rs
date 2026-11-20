use std::time::Duration;

use getset::CopyGetters;
use muster_domain::value::Pid;

use super::retry_id::RetryId;
use crate::backoff::Backoff;

/// One pane's retries of adopting the holder of a child left running
/// unsupervised, owned and built by [`super::adoption_retries::AdoptionRetries`].
#[derive(Debug, Clone, Copy, CopyGetters)]
pub(super) struct PaneRetry {
    /// The holder whose failed adoption was already told, so a retry that
    /// fails the same way is not told again.
    told: Option<Pid>,
    /// The waits before each timed retry.
    backoff: Backoff,
    /// The current try; a result or timer of any other is stale.
    #[getset(get_copy = "pub(super)")]
    current: RetryId,
}

impl PaneRetry {
    /// A pane's first retries, whose timed retries wait as `backoff` says.
    pub(super) fn new(backoff: Backoff) -> Self {
        Self {
            told: None,
            backoff,
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
    /// backing off the wait of the next one.
    pub(super) fn schedule(&mut self, id: RetryId) -> Duration {
        self.current = id;
        self.backoff.step()
    }
}
