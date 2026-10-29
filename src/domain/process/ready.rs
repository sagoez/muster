use std::fmt;

use getset::Getters;
use nutype::nutype;
use strum::{Display, EnumIter, EnumString};
use typed_builder::TypedBuilder;

use crate::domain::value::{LocalPort, LocalUrl, PositiveDuration, ValueParseError, parse_choice};

/// Longest a log pattern may be, in characters, so no agent can hand the
/// service a pattern that stalls it.
pub const PATTERN_MAX_CHARS: usize = 512;
/// What a pattern's text must be.
const PATTERN_FORM: &str = "a pattern of 1 to 512 characters";

/// The text of a log pattern: trimmed, 1 to [`PATTERN_MAX_CHARS`] characters.
#[nutype(
    sanitize(trim),
    validate(not_empty, len_char_max = PATTERN_MAX_CHARS),
    derive(Debug, Clone, PartialEq, Eq, Hash, AsRef, Display, Serialize, Deserialize)
)]
pub struct PatternText(String);

/// How a log pattern is matched against one output line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Display, EnumString)]
#[strum(serialize_all = "kebab-case")]
pub enum PatternKind {
    /// The line contains the text.
    Substring,
    /// The line matches the text as a regular expression.
    Regex,
}

impl PatternKind {
    /// A regex when `regex`, else a substring.
    pub fn of_regex(regex: bool) -> Self {
        if regex { Self::Regex } else { Self::Substring }
    }
}

/// A pattern one output line is tested against.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct LogPattern {
    /// What to look for.
    text: PatternText,
    /// How to look for it.
    kind: PatternKind,
}

impl fmt::Display for LogPattern {
    /// Writes the text, with `(regex)` after a regex.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.kind {
            PatternKind::Substring => write!(formatter, "{}", self.text),
            PatternKind::Regex => write!(formatter, "{} ({})", self.text, self.kind),
        }
    }
}

/// How a readiness check is stored: the kind column of a process row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Display, EnumString, EnumIter)]
#[strum(serialize_all = "kebab-case")]
pub enum ReadyKind {
    /// A substring of one output line.
    Log,
    /// A regex over one output line.
    Regex,
    /// A TCP connect to a local port.
    Port,
    /// An HTTP response below 500 from a local URL.
    Http,
}

/// What a readiness check waits for.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ReadyProbe {
    /// An output line matching the pattern.
    Log(LogPattern),
    /// A TCP connection accepted on the local port.
    Port(LocalPort),
    /// An HTTP response below 500 from the local URL.
    Http(LocalUrl),
}

impl ReadyProbe {
    /// The probe stored as `kind` with `target`.
    ///
    /// # Errors
    /// Returns [`ValueParseError`] when `kind` is unknown or `target` does not
    /// suit it.
    pub fn from_stored(kind: &str, target: &str) -> Result<Self, ValueParseError> {
        let pattern = |kind| {
            PatternText::try_new(target)
                .map(|text| Self::Log(LogPattern::builder().text(text).kind(kind).build()))
                .map_err(|_| {
                    ValueParseError::builder()
                        .expected(PATTERN_FORM)
                        .found(target)
                        .build()
                })
        };
        match parse_choice::<ReadyKind>(kind)? {
            ReadyKind::Log => pattern(PatternKind::Substring),
            ReadyKind::Regex => pattern(PatternKind::Regex),
            ReadyKind::Port => target.parse().map(Self::Port),
            ReadyKind::Http => target.parse().map(Self::Http),
        }
    }

    /// The local port this probe needs its process to listen on, when it
    /// names one.
    pub fn port(&self) -> Option<LocalPort> {
        match self {
            Self::Log(_) => None,
            Self::Port(port) => Some(*port),
            Self::Http(url) => url.port(),
        }
    }

    /// The kind and target this probe is stored as, which
    /// [`Self::from_stored`] reads back.
    pub fn stored(&self) -> (ReadyKind, String) {
        match self {
            Self::Log(pattern) => (
                match pattern.kind() {
                    PatternKind::Substring => ReadyKind::Log,
                    PatternKind::Regex => ReadyKind::Regex,
                },
                pattern.text().to_string(),
            ),
            Self::Port(port) => (ReadyKind::Port, port.to_string()),
            Self::Http(url) => (ReadyKind::Http, url.to_string()),
        }
    }
}

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
