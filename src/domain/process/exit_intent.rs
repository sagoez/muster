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
    /// Start a fresh child regardless of the configured restart policy.
    Restart,
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
        Self::Restart
    }

    /// Whether another stop input should attempt signal delivery.
    pub fn accepts_stop_request(self) -> bool {
        self != Self::StopInFlight
    }

    /// Whether this intent represents an explicit request to remain stopped.
    pub fn is_stop(self) -> bool {
        matches!(self, Self::StopRetryable | Self::StopInFlight)
    }

    /// Whether a graceful stop is awaiting hard-kill escalation.
    pub fn awaits_force_stop(self) -> bool {
        self == Self::StopInFlight
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

        assert_eq!(intent, ExitIntent::Restart);
        assert!(!intent.is_stop());
    }
}
