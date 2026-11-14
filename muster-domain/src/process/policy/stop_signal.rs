use strum::{Display, EnumIter, EnumString};

/// Graceful signal sent before a command is force-killed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Display, EnumString, EnumIter)]
#[strum(serialize_all = "kebab-case")]
pub enum StopSignal {
    /// Requests the conventional service shutdown path (SIGTERM on Unix).
    Terminate,
    /// Requests the conventional interactive interrupt path (SIGINT on Unix).
    Interrupt,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_signals_by_kebab_case_name() {
        assert_eq!(
            "interrupt".parse::<StopSignal>().unwrap(),
            StopSignal::Interrupt
        );
        assert!("kill".parse::<StopSignal>().is_err());
    }
}
