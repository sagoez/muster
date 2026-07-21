/// Desired handling of the current child process when it exits.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ExitIntent {
    /// Apply the configured restart policy to an ordinary exit.
    #[default]
    FollowPolicy,
    /// Keep the process stopped, but permit another signal attempt because no
    /// successful delivery is currently in flight.
    StopRetryable,
    /// Keep the process stopped and reuse the successfully delivered request.
    StopInFlight,
    /// Restart after exit, but permit another signal attempt because no
    /// successful delivery is currently in flight.
    RestartRetryable,
    /// Restart after exit and reuse the successfully delivered request.
    RestartInFlight,
}

impl ExitIntent {
    /// Records explicit stop intent before attempting signal delivery.
    pub fn request_stop(self) -> Self {
        Self::StopRetryable
    }

    /// Records that a stop signal was delivered successfully.
    pub fn stop_delivered(self) -> Self {
        if self.is_stop() {
            Self::StopInFlight
        } else {
            self
        }
    }

    /// Records failed stop delivery, permitting a later retry while preserving
    /// the user's instruction not to restart when the child exits.
    pub fn stop_delivery_failed(self) -> Self {
        if self.is_stop() {
            Self::StopRetryable
        } else {
            self
        }
    }

    /// Replaces any prior exit intent with an explicit restart request.
    pub fn request_restart(self) -> Self {
        Self::RestartRetryable
    }

    /// Records that a restart signal was delivered successfully.
    pub fn restart_delivered(self) -> Self {
        if self.is_restart() {
            Self::RestartInFlight
        } else {
            self
        }
    }

    /// Records failed restart delivery, permitting a later retry while
    /// preserving the user's instruction to restart after any eventual exit.
    pub fn restart_delivery_failed(self) -> Self {
        if self.is_restart() {
            Self::RestartRetryable
        } else {
            self
        }
    }

    /// Whether another stop input should attempt signal delivery.
    pub fn accepts_stop_request(self) -> bool {
        self != Self::StopInFlight
    }

    /// Whether another restart input should attempt signal delivery.
    pub fn accepts_restart_request(self) -> bool {
        self != Self::RestartInFlight
    }

    /// Whether this intent represents an explicit request to remain stopped.
    pub fn is_stop(self) -> bool {
        matches!(self, Self::StopRetryable | Self::StopInFlight)
    }

    /// Whether this intent represents an explicit restart request.
    pub fn is_restart(self) -> bool {
        matches!(self, Self::RestartRetryable | Self::RestartInFlight)
    }

    /// Records that hard-kill escalation failed, preserving the requested exit
    /// behavior while permitting another signal attempt.
    pub fn force_stop_delivery_failed(self) -> Self {
        match self {
            Self::StopInFlight => Self::StopRetryable,
            Self::RestartInFlight => Self::RestartRetryable,
            _ => self,
        }
    }

    /// Whether a graceful stop is awaiting hard-kill escalation.
    pub fn awaits_force_stop(self) -> bool {
        matches!(self, Self::StopInFlight | Self::RestartInFlight)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_delivery_preserves_stop_intent_and_allows_retry() {
        let intent = ExitIntent::default().request_stop().stop_delivery_failed();

        assert!(intent.is_stop());
        assert!(intent.accepts_stop_request());
        assert!(!intent.awaits_force_stop());
    }

    #[test]
    fn successful_delivery_suppresses_duplicate_stop_requests() {
        let intent = ExitIntent::default().request_stop().stop_delivered();

        assert!(intent.is_stop());
        assert!(!intent.accepts_stop_request());
        assert!(intent.awaits_force_stop());
    }

    #[test]
    fn restart_replaces_a_prior_stop_intent() {
        let intent = ExitIntent::default()
            .request_stop()
            .stop_delivered()
            .request_restart();

        assert_eq!(intent, ExitIntent::RestartRetryable);
        assert!(!intent.is_stop());
        assert!(intent.is_restart());
    }

    /// A delivered restart waits for exit and permits timeout escalation.
    #[test]
    fn successful_restart_delivery_awaits_force_stop() {
        let intent = ExitIntent::default().request_restart().restart_delivered();

        assert_eq!(intent, ExitIntent::RestartInFlight);
        assert!(!intent.accepts_restart_request());
        assert!(intent.awaits_force_stop());
    }

    /// Failed escalation keeps the user's restart intent retryable.
    #[test]
    fn failed_restart_escalation_remains_retryable() {
        let intent = ExitIntent::RestartInFlight.force_stop_delivery_failed();

        assert_eq!(intent, ExitIntent::RestartRetryable);
        assert!(intent.accepts_restart_request());
    }
}
