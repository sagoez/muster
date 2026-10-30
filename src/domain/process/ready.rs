use getset::Getters;
use typed_builder::TypedBuilder;

use super::ready_probe::ReadyProbe;
use crate::domain::value::PositiveDuration;

/// A process's stored readiness check: what to wait for after each spawn, and
/// how long before it counts as not ready.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct ReadyCheck {
    /// What to wait for.
    probe: ReadyProbe,
    /// How long a run has to pass it.
    timeout: PositiveDuration,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{
        process::{LogPattern, PATTERN_MAX_CHARS, PatternKind, PatternText},
        value::LocalPort,
    };

    #[test]
    fn a_pattern_is_trimmed_and_bounded() {
        assert_eq!(
            PatternText::try_new("  Listening ").unwrap().as_ref(),
            "Listening"
        );
        assert!(PatternText::try_new("   ").is_err());
        assert!(PatternText::try_new("x".repeat(PATTERN_MAX_CHARS)).is_ok());
        assert!(PatternText::try_new("x".repeat(PATTERN_MAX_CHARS + 1)).is_err());
    }

    #[test]
    fn every_kind_reads_its_stored_target() {
        let pattern = |text: &str, kind| {
            ReadyProbe::Log(
                LogPattern::builder()
                    .text(PatternText::try_new(text).unwrap())
                    .kind(kind)
                    .build(),
            )
        };

        assert_eq!(
            ReadyProbe::from_stored("log", "Listening on").unwrap(),
            pattern("Listening on", PatternKind::Substring)
        );
        assert_eq!(
            ReadyProbe::from_stored("regex", r"ready in \d+ms").unwrap(),
            pattern(r"ready in \d+ms", PatternKind::Regex)
        );
        assert_eq!(
            ReadyProbe::from_stored("port", "3000").unwrap(),
            ReadyProbe::Port(LocalPort::try_new(3000).unwrap())
        );
        assert_eq!(
            ReadyProbe::from_stored("http", "http://localhost:3000/health").unwrap(),
            ReadyProbe::Http("http://localhost:3000/health".parse().unwrap())
        );
    }

    #[test]
    fn every_probe_reads_back_as_stored() {
        for (kind, target) in [
            ("log", "Listening on"),
            ("regex", r"ready in \d+ms"),
            ("port", "3000"),
            ("http", "http://localhost:3000/health"),
        ] {
            let probe = ReadyProbe::from_stored(kind, target).unwrap();
            let (stored_kind, stored_target) = probe.stored();
            assert_eq!(
                ReadyProbe::from_stored(&stored_kind.to_string(), &stored_target).unwrap(),
                probe
            );
            assert_eq!(stored_kind.to_string(), kind);
        }
    }

    #[test]
    fn a_stored_probe_is_checked_against_its_kind() {
        assert!(ReadyProbe::from_stored("port", "0").is_err());
        assert!(ReadyProbe::from_stored("http", "http://example.com").is_err());
        assert!(ReadyProbe::from_stored("tcp", "3000").is_err());
        assert!(ReadyProbe::from_stored("log", " ").is_err());
    }
}
