use std::fmt;

use getset::Getters;
use typed_builder::TypedBuilder;

use super::{pattern_kind::PatternKind, pattern_text::PatternText};

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
