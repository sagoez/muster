use super::{
    log_pattern::LogPattern,
    pattern_kind::PatternKind,
    pattern_text::{PATTERN_FORM, PatternText},
    ready_kind::ReadyKind,
};
use crate::domain::value::{LocalPort, LocalUrl, ValueParseError, parse_choice};

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
