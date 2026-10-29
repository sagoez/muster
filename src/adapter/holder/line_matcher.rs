use regex::{Regex, RegexBuilder};

use super::pattern_error::PatternError;
use crate::domain::process::{LogPattern, PatternKind};

/// The most memory a compiled regex may take, so a pathological pattern is
/// refused rather than stalling whoever compiles it.
pub const REGEX_SIZE_LIMIT: usize = 1024 * 1024;

/// How a compiled matcher tests a line.
#[derive(Debug, Clone)]
enum Matcher {
    /// The line contains the text.
    Substring(String),
    /// The regex is found in the line.
    Regex(Regex),
}

/// A [`LogPattern`] compiled once and tested against many stripped lines.
/// Regexes run in linear time, so no line can make a match slow.
#[derive(Debug, Clone)]
pub struct LineMatcher(Matcher);

impl LineMatcher {
    /// Compiles `pattern`, bounding a regex's compiled size by
    /// [`REGEX_SIZE_LIMIT`].
    ///
    /// # Errors
    /// Returns [`PatternError::InvalidRegex`] for a regex that does not parse
    /// or compiles too large.
    pub fn compile(pattern: &LogPattern) -> Result<Self, PatternError> {
        let text = pattern.text().as_ref();
        let matcher = match pattern.kind() {
            PatternKind::Substring => Matcher::Substring(text.to_string()),
            PatternKind::Regex => Matcher::Regex(
                RegexBuilder::new(text)
                    .size_limit(REGEX_SIZE_LIMIT)
                    .build()
                    .map_err(|error| PatternError::InvalidRegex {
                        pattern: text.to_string(),
                        reason: error.to_string(),
                    })?,
            ),
        };
        Ok(Self(matcher))
    }

    /// Whether `line` matches.
    pub fn is_match(&self, line: &str) -> bool {
        match &self.0 {
            Matcher::Substring(text) => line.contains(text.as_str()),
            Matcher::Regex(regex) => regex.is_match(line),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::process::PatternText;

    fn pattern(text: &str, kind: PatternKind) -> LogPattern {
        LogPattern::builder()
            .text(PatternText::try_new(text).unwrap())
            .kind(kind)
            .build()
    }

    #[test]
    fn substrings_and_regexes_match_lines() {
        let substring =
            LineMatcher::compile(&pattern("Listening on", PatternKind::Substring)).unwrap();
        assert!(substring.is_match("Listening on http://localhost:3000"));
        assert!(!substring.is_match("listening on"));
        assert!(
            !LineMatcher::compile(&pattern("a.c", PatternKind::Substring))
                .unwrap()
                .is_match("abc")
        );

        let regex = LineMatcher::compile(&pattern(r"ready in \d+ms", PatternKind::Regex)).unwrap();
        assert!(regex.is_match("vite ready in 312ms"));
        assert!(!regex.is_match("ready in ms"));
    }

    /// A regex that does not parse, or would compile past the size limit, is
    /// refused with the reason.
    #[test]
    fn unusable_regexes_are_refused() {
        let error = LineMatcher::compile(&pattern("(unclosed", PatternKind::Regex)).unwrap_err();
        assert!(
            error
                .to_string()
                .starts_with("'(unclosed' is not a usable regex"),
            "{error}"
        );
        assert!(LineMatcher::compile(&pattern(r"\w{1000}{1000}", PatternKind::Regex)).is_err());
    }
}
