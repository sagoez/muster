use std::{collections::HashMap, time::Duration};

use crate::domain::value::{PaneId, Pid};

/// Wait before the first timed retry of an adoption that failed.
const FIRST_RETRY: Duration = Duration::from_secs(30);
/// Longest wait between timed retries of an adoption that keeps failing.
const MAX_RETRY: Duration = Duration::from_secs(10 * 60);

/// One pane's retries of adopting the holder of a child left running
/// unsupervised.
#[derive(Debug, Clone, Copy)]
struct PaneRetry {
    /// The holder whose failed adoption was already told, so a retry that
    /// fails the same way is not told again.
    told: Option<Pid>,
    /// Wait before the next timed retry.
    delay: Duration,
    /// The current try; a result or timer of any other is stale.
    current: u64,
}

/// The retries of every pane left unsupervised by a failed adoption: each
/// tries again on a timer that backs off, or when a start asks, and the
/// failure of one holder is told once however often it repeats.
#[derive(Debug, Default)]
pub(super) struct AdoptionRetries {
    /// Each pane's retries, while it has any.
    panes: HashMap<PaneId, PaneRetry>,
    /// The id the next try takes, unique across panes.
    next: u64,
}

impl AdoptionRetries {
    /// Whether the failed adoption of `holder` for `pane` is news, recording
    /// that it was told.
    pub(super) fn first_failure(&mut self, pane: PaneId, holder: Pid) -> bool {
        let retry = self.entry(pane);
        let news = retry.told != Some(holder);
        retry.told = Some(holder);
        news
    }

    /// Starts a new try of `pane` now, superseding any scheduled one, and
    /// returns its id.
    pub(super) fn begin(&mut self, pane: PaneId) -> u64 {
        let id = self.take_id();
        self.entry(pane).current = id;
        id
    }

    /// Schedules the next timed try of `pane`, superseding any other: returns
    /// its id and how long to wait, which doubles each time up to
    /// [`MAX_RETRY`].
    pub(super) fn schedule(&mut self, pane: PaneId) -> (u64, Duration) {
        let id = self.take_id();
        let retry = self.entry(pane);
        let delay = retry.delay;
        retry.delay = retry.delay.saturating_mul(2).min(MAX_RETRY);
        retry.current = id;
        (id, delay)
    }

    /// Whether `id` is `pane`'s current try.
    pub(super) fn is_current(&self, pane: PaneId, id: u64) -> bool {
        self.panes
            .get(&pane)
            .is_some_and(|retry| retry.current == id)
    }

    /// `pane`'s current try, for a test to let it come due.
    #[cfg(test)]
    pub(super) fn current(&self, pane: PaneId) -> Option<u64> {
        self.panes.get(&pane).map(|retry| retry.current)
    }

    /// Forgets `pane`'s retries, once its holder is supervised again.
    pub(super) fn forget(&mut self, pane: PaneId) {
        self.panes.remove(&pane);
    }

    /// `pane`'s retries, fresh when it has none.
    fn entry(&mut self, pane: PaneId) -> &mut PaneRetry {
        self.panes.entry(pane).or_insert(PaneRetry {
            told: None,
            delay: FIRST_RETRY,
            current: 0,
        })
    }

    /// A try id no try has had.
    fn take_id(&mut self) -> u64 {
        self.next += 1;
        self.next
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The pane every retry here is of.
    fn pane() -> PaneId {
        PaneId::new(0)
    }

    /// A holder's failure is told once, a different holder's again, and each
    /// timed retry waits twice as long as the last, up to the cap, the newest
    /// alone current.
    #[test]
    fn a_failure_is_told_once_per_holder_and_retries_back_off() {
        let mut retries = AdoptionRetries::default();
        let holder = Pid::try_new(7).unwrap();
        let other = Pid::try_new(8).unwrap();

        assert!(retries.first_failure(pane(), holder));
        assert!(!retries.first_failure(pane(), holder));
        assert!(retries.first_failure(pane(), other));

        let (first, wait) = retries.schedule(pane());
        assert_eq!(wait, FIRST_RETRY);
        let (second, wait) = retries.schedule(pane());
        assert_eq!(wait, FIRST_RETRY * 2);
        assert!(!retries.is_current(pane(), first));
        assert!(retries.is_current(pane(), second));
        for _ in 0..16 {
            retries.schedule(pane());
        }
        assert_eq!(retries.schedule(pane()).1, MAX_RETRY);
        let asked = retries.begin(pane());
        assert!(retries.is_current(pane(), asked));
    }
}
