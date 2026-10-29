use strum::{Display, EnumString};

use super::ReadyCheck;

/// Whether one run of a process has passed its readiness check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Display, EnumString)]
#[strum(serialize_all = "kebab-case")]
pub enum Readiness {
    /// No check, or no run to check.
    #[default]
    None,
    /// The run has not passed its check yet.
    Waiting,
    /// The run passed its check.
    Ready,
    /// The run did not pass its check in time; it keeps running.
    TimedOut,
}

impl Readiness {
    /// The readiness a fresh run starts with: waiting when it has a check.
    pub fn on_spawn(check: Option<&ReadyCheck>) -> Self {
        match check {
            Some(_) => Self::Waiting,
            None => Self::None,
        }
    }

    /// The readiness once the check passed.
    #[must_use]
    pub fn on_passed(self) -> Self {
        match self {
            Self::Waiting => Self::Ready,
            other => other,
        }
    }

    /// The readiness once the check's time ran out.
    #[must_use]
    pub fn on_deadline(self) -> Self {
        match self {
            Self::Waiting => Self::TimedOut,
            other => other,
        }
    }

    /// The readiness once the run ended.
    #[must_use]
    pub fn on_exit(self) -> Self {
        Self::None
    }

    /// Whether the run still waits on its check.
    pub fn is_waiting(self) -> bool {
        self == Self::Waiting
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::domain::{
        process::ReadyProbe,
        value::{LocalPort, PositiveDuration},
    };

    fn check() -> ReadyCheck {
        ReadyCheck::builder()
            .probe(ReadyProbe::Port(LocalPort::try_new(3000).unwrap()))
            .timeout(PositiveDuration::try_new(Duration::from_secs(30)).unwrap())
            .build()
    }

    #[test]
    fn a_run_with_a_check_waits_and_one_without_has_none() {
        assert_eq!(Readiness::on_spawn(Some(&check())), Readiness::Waiting);
        assert_eq!(Readiness::on_spawn(None), Readiness::None);
    }

    #[test]
    fn only_a_waiting_run_passes_or_times_out() {
        assert_eq!(Readiness::Waiting.on_passed(), Readiness::Ready);
        assert_eq!(Readiness::Waiting.on_deadline(), Readiness::TimedOut);
        assert_eq!(Readiness::Ready.on_deadline(), Readiness::Ready);
        assert_eq!(Readiness::TimedOut.on_passed(), Readiness::TimedOut);
        assert_eq!(Readiness::None.on_passed(), Readiness::None);
    }

    #[test]
    fn an_exit_clears_it() {
        for readiness in [
            Readiness::Waiting,
            Readiness::Ready,
            Readiness::TimedOut,
            Readiness::None,
        ] {
            assert_eq!(readiness.on_exit(), Readiness::None);
        }
    }
}
