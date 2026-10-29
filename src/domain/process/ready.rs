use getset::{CopyGetters, Getters};
use nutype::nutype;
use typed_builder::TypedBuilder;

/// The longest pattern a log match or readiness check may use, so an agent
/// cannot hand the service a pattern that is costly to compile.
pub const PATTERN_MAX_CHARS: usize = 512;

/// The text of a log pattern: trimmed, 1 to [`PATTERN_MAX_CHARS`] characters.
#[nutype(
    sanitize(trim),
    validate(not_empty, len_char_max = PATTERN_MAX_CHARS),
    derive(Debug, Clone, PartialEq, Eq, Hash, AsRef, Display, Serialize, Deserialize)
)]
pub struct PatternText(String);

/// How a log pattern's text is matched against one output line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PatternKind {
    /// The line contains the text.
    Substring,
    /// The text is a regular expression found in the line.
    Regex,
}

/// A pattern matched against single lines of a process's output.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Getters, CopyGetters, TypedBuilder)]
pub struct LogPattern {
    /// The pattern's text.
    #[getset(get = "pub")]
    text: PatternText,
    /// How the text is matched.
    #[getset(get_copy = "pub")]
    kind: PatternKind,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pattern_text_is_trimmed_and_bounded() {
        assert_eq!(
            PatternText::try_new("  Listening on ").unwrap().as_ref(),
            "Listening on"
        );
        assert!(PatternText::try_new("   ").is_err());
        assert!(PatternText::try_new("x".repeat(PATTERN_MAX_CHARS)).is_ok());
        assert!(PatternText::try_new("é".repeat(PATTERN_MAX_CHARS + 1)).is_err());
    }
}
