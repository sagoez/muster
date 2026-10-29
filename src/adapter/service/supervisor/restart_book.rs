use std::collections::{HashMap, HashSet};

use super::run_end::RunEnd;
use crate::{
    adapter::service::{ExitReason, LastExit},
    domain::{
        pty::{ExitOutcome, SpawnHistory},
        value::PaneId,
    },
};

/// Each pane's restarts and crashes: the backoff of its automatic restarts,
/// the restarts `ps` shows, how its last run ended, and the streaks that keep
/// a repeating failure told once.
#[derive(Default)]
pub(crate) struct RestartBook {
    /// Consecutive quick automatic restarts, which set the backoff delay.
    attempts: HashMap<PaneId, u32>,
    /// Restarts of each pane, by its policy or on request.
    restarts: HashMap<PaneId, u32>,
    /// How each pane's most recent child ended, shown by `ps` while it is
    /// not running.
    last_exits: HashMap<PaneId, ExitOutcome>,
    /// How each pane's most recent run ended, while this service saw it.
    run_ends: HashMap<PaneId, RunEnd>,
    /// Panes in a crash streak, whose first crash was told and whose later
    /// ones are not, until the streak ends.
    crash_streaks: HashSet<PaneId>,
    /// Panes whose last spawn failed, so a failure streak is reported once.
    failing_spawns: HashSet<PaneId>,
}

impl RestartBook {
    /// How many times `pane` was restarted.
    pub(super) fn restarts(&self, pane: PaneId) -> u32 {
        self.restarts.get(&pane).copied().unwrap_or(0)
    }

    /// Counts one more restart of `pane`.
    pub(super) fn count_restart(&mut self, pane: PaneId) {
        *self.restarts.entry(pane).or_default() += 1;
    }

    /// The history a spawn of `pane` carries to its holder, counting the
    /// spawn itself when it is a restart.
    pub(super) fn history(&self, pane: PaneId, restarting: bool) -> SpawnHistory {
        SpawnHistory::builder()
            .restarts(self.restarts(pane) + u32::from(restarting))
            .last_exit(self.last_exits.get(&pane).cloned())
            .build()
    }

    /// Takes over the restart count and last exit `history` carries from the
    /// service that spawned `pane`'s child.
    pub(super) fn take_history(&mut self, pane: PaneId, history: &SpawnHistory) {
        if history.restarts() > 0 {
            self.restarts.insert(pane, history.restarts());
        }
        if let Some(last_exit) = history.last_exit() {
            self.last_exits.insert(pane, last_exit.clone());
        }
    }

    /// Records how `pane`'s child ended.
    pub(super) fn record_exit(&mut self, pane: PaneId, outcome: ExitOutcome) {
        self.last_exits.insert(pane, outcome);
    }

    /// Records how `pane`'s run ended, as this service saw it.
    pub(super) fn end_run(&mut self, pane: PaneId, end: RunEnd) {
        self.run_ends.insert(pane, end);
    }

    /// How `pane`'s most recent run ended, while this service saw it.
    pub(super) fn run_end(&self, pane: PaneId) -> Option<RunEnd> {
        self.run_ends.get(&pane).copied()
    }

    /// How `pane`'s most recent run ended: why, as this service saw it, or as
    /// its outcome tells for one an earlier service saw.
    pub(super) fn last_exit(&self, pane: PaneId) -> Option<LastExit> {
        let end = self.run_ends.get(&pane);
        let outcome = match end {
            Some(end) if end.reason() == ExitReason::FailedToStart => ExitOutcome::FAILED_UNKNOWN,
            Some(_) | None => self.last_exits.get(&pane)?.clone(),
        };
        Some(
            LastExit::builder()
                .reason(end.map_or_else(|| ExitReason::of(&outcome, false), RunEnd::reason))
                .outcome(outcome)
                .at(end.map(RunEnd::at))
                .build(),
        )
    }

    /// The backoff exponent of `pane`'s next automatic restart, counting it:
    /// zero after a stable run, else one more than the last.
    pub(super) fn next_attempt(&mut self, pane: PaneId, stable: bool) -> u32 {
        let attempts = if stable {
            0
        } else {
            self.attempts.get(&pane).copied().unwrap_or(0)
        };
        self.attempts.insert(pane, attempts + 1);
        attempts
    }

    /// Resets `pane`'s backoff, for a restart that is not quick.
    pub(super) fn reset_backoff(&mut self, pane: PaneId) {
        self.attempts.remove(&pane);
    }

    /// Whether `pane` is backing off between automatic restarts.
    #[cfg(test)]
    pub(super) fn backs_off(&self, pane: PaneId) -> bool {
        self.attempts.contains_key(&pane)
    }

    /// Ends `pane`'s crash streak, so its next crash is told.
    pub(super) fn end_crash_streak(&mut self, pane: PaneId) {
        self.crash_streaks.remove(&pane);
    }

    /// Ends both of `pane`'s streaks, for a start the user asked for.
    pub(super) fn end_streaks(&mut self, pane: PaneId) {
        self.failing_spawns.remove(&pane);
        self.crash_streaks.remove(&pane);
    }

    /// Counts a crash of `pane`; returns whether it begins a streak, and so
    /// is told.
    pub(super) fn crashed(&mut self, pane: PaneId) -> bool {
        self.crash_streaks.insert(pane)
    }

    /// Ends `pane`'s spawn failure streak, for a spawn that worked.
    pub(super) fn spawned(&mut self, pane: PaneId) {
        self.failing_spawns.remove(&pane);
    }

    /// Counts a failed spawn of `pane`; returns whether it begins a streak,
    /// and so is told.
    pub(super) fn spawn_failed(&mut self, pane: PaneId) -> bool {
        self.failing_spawns.insert(pane)
    }

    /// Forgets everything about `pane`, retired from the workspace.
    pub(super) fn forget(&mut self, pane: PaneId) {
        self.attempts.remove(&pane);
        self.restarts.remove(&pane);
        self.last_exits.remove(&pane);
        self.run_ends.remove(&pane);
        self.crash_streaks.remove(&pane);
        self.failing_spawns.remove(&pane);
    }
}

#[cfg(test)]
mod tests {
    use std::time::SystemTime;

    use super::*;
    use crate::adapter::service::supervisor::SpawnGeneration;

    const PANE: u64 = 0;

    /// Quick restarts back off further each time; a stable run starts over.
    #[test]
    fn the_backoff_grows_until_a_stable_run_resets_it() {
        let mut book = RestartBook::default();
        let pane = PaneId::new(PANE);

        assert_eq!(book.next_attempt(pane, false), 0);
        assert_eq!(book.next_attempt(pane, false), 1);
        assert_eq!(book.next_attempt(pane, true), 0);
        book.reset_backoff(pane);
        assert!(!book.backs_off(pane));
    }

    /// A crash streak is told once, until it ends.
    #[test]
    fn a_crash_streak_is_told_once_until_it_ends() {
        let mut book = RestartBook::default();
        let pane = PaneId::new(PANE);

        assert!(book.crashed(pane));
        assert!(!book.crashed(pane));
        book.end_streaks(pane);
        assert!(book.crashed(pane));
    }

    /// A run that failed to start reads as an unknown failure, whatever exit
    /// an earlier run left.
    #[test]
    fn a_failed_start_reads_as_an_unknown_failure() {
        let mut book = RestartBook::default();
        let pane = PaneId::new(PANE);
        book.record_exit(pane, ExitOutcome::Succeeded);
        book.end_run(
            pane,
            RunEnd::builder()
                .generation(SpawnGeneration::initial())
                .at(SystemTime::now())
                .reason(ExitReason::FailedToStart)
                .build(),
        );

        let last = book.last_exit(pane).unwrap();

        assert_eq!(last.outcome(), &ExitOutcome::FAILED_UNKNOWN);
        assert_eq!(last.reason(), ExitReason::FailedToStart);
    }
}
