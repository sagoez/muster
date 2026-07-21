use std::time::{Duration, Instant};

use crate::domain::process::ActivityState;

/// Quiet period after ordinary output before a process returns from working to
/// idle. Explicit progress remains working until its protocol reports completion.
pub(super) const OUTPUT_IDLE_TIMEOUT: Duration = Duration::from_secs(1);

/// Evidence currently governing inferred process activity. The variants prevent
/// an ordinary-output deadline and explicit protocol progress from coexisting.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum ActivityTracker {
    /// No inferred timeout is scheduled. The displayed activity may be idle or
    /// awaiting input, as owned by the process domain value.
    #[default]
    Unscheduled,
    /// Ordinary output keeps the process working until this observation expires.
    RecentOutput(Instant),
    /// Explicit protocol progress keeps the process working until completion.
    ExplicitProgress,
}

impl ActivityTracker {
    /// Records ordinary terminal output and returns the resulting activity.
    pub(super) fn observe_output(&mut self, now: Instant) -> ActivityState {
        if *self != Self::ExplicitProgress {
            *self = Self::RecentOutput(now);
        }
        ActivityState::Working
    }

    /// Records a protocol progress transition and returns the resulting activity.
    pub(super) fn observe_progress(&mut self, active: bool) -> ActivityState {
        if active {
            *self = Self::ExplicitProgress;
            ActivityState::Working
        } else {
            *self = Self::Unscheduled;
            ActivityState::AwaitingInput
        }
    }

    /// Records a notification or bell and returns the resulting activity.
    pub(super) fn observe_attention(&mut self) -> ActivityState {
        *self = Self::Unscheduled;
        ActivityState::AwaitingInput
    }

    /// Clears all inferred activity for a new or exited child.
    pub(super) fn reset(&mut self) {
        *self = Self::Unscheduled;
    }

    /// Returns when recent ordinary output should become idle, if scheduled.
    pub(super) fn deadline(self) -> Option<Instant> {
        match self {
            Self::RecentOutput(observed_at) => Some(observed_at + OUTPUT_IDLE_TIMEOUT),
            Self::Unscheduled | Self::ExplicitProgress => None,
        }
    }

    /// Expires recent ordinary output at `now`, returning the new activity only
    /// when a transition to idle occurred.
    pub(super) fn expire(&mut self, now: Instant) -> Option<ActivityState> {
        if self.deadline().is_some_and(|deadline| deadline <= now) {
            *self = Self::Unscheduled;
            Some(ActivityState::Idle)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_output_expires_to_idle() {
        let observed_at = Instant::now();
        let mut tracker = ActivityTracker::default();

        assert_eq!(tracker.observe_output(observed_at), ActivityState::Working);
        assert_eq!(
            tracker.expire(observed_at + OUTPUT_IDLE_TIMEOUT),
            Some(ActivityState::Idle)
        );
        assert_eq!(tracker, ActivityTracker::Unscheduled);
    }

    #[test]
    fn explicit_progress_has_no_ordinary_output_deadline() {
        let mut tracker = ActivityTracker::default();
        tracker.observe_progress(true);
        tracker.observe_output(Instant::now());

        assert_eq!(tracker, ActivityTracker::ExplicitProgress);
        assert!(tracker.deadline().is_none());
        assert_eq!(tracker.expire(Instant::now() + OUTPUT_IDLE_TIMEOUT), None);
    }

    #[test]
    fn attention_clears_an_ordinary_output_deadline() {
        let mut tracker = ActivityTracker::default();
        tracker.observe_output(Instant::now());

        assert_eq!(tracker.observe_attention(), ActivityState::AwaitingInput);
        assert_eq!(tracker, ActivityTracker::Unscheduled);
    }
}
