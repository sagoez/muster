use thiserror::Error;

use super::refusal::{IDLE_STOP_MISMATCHED, NOT_POSITIVE_MILLIS};

/// Why stored idle-stop columns do not read as an idle stop.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub(super) enum IdleStopRefusal {
    /// The kind is unknown, or its milliseconds do not go with it.
    #[error("{IDLE_STOP_MISMATCHED}")]
    Mismatched,
    /// The milliseconds are not a positive whole number.
    #[error("{NOT_POSITIVE_MILLIS}")]
    NotPositive,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::store::refusal::Refusal;

    /// Each refusal reads as the same sentence it gives a stored-row error.
    #[test]
    fn each_refusal_displays_its_reason() {
        for refusal in [IdleStopRefusal::Mismatched, IdleStopRefusal::NotPositive] {
            assert_eq!(refusal.to_string(), refusal.reason());
        }
    }
}
