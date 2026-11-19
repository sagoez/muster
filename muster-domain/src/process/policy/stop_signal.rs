use strum::{Display, EnumIter, EnumMessage, EnumString};

/// Graceful signal sent before a command is force-killed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Display, EnumString, EnumIter, EnumMessage)]
#[strum(serialize_all = "kebab-case")]
pub enum StopSignal {
    /// Requests the conventional service shutdown path (SIGTERM on Unix).
    #[strum(message = "SIGTERM")]
    Terminate,
    /// Requests the conventional interactive interrupt path (SIGINT on Unix).
    #[strum(message = "SIGINT")]
    Interrupt,
}

impl StopSignal {
    /// The Unix name of the signal sent, such as `SIGTERM`.
    pub fn unix_name(self) -> &'static str {
        self.get_message().unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use strum::IntoEnumIterator;

    use super::*;

    #[test]
    fn reads_signals_by_kebab_case_name() {
        assert_eq!(
            "interrupt".parse::<StopSignal>().unwrap(),
            StopSignal::Interrupt
        );
        assert!("kill".parse::<StopSignal>().is_err());
    }

    /// Every signal names the Unix signal it sends.
    #[test]
    fn every_signal_has_its_unix_name() {
        assert_eq!(StopSignal::Terminate.unix_name(), "SIGTERM");
        assert_eq!(StopSignal::Interrupt.unix_name(), "SIGINT");
        assert!(StopSignal::iter().all(|signal| !signal.unix_name().is_empty()));
    }
}
