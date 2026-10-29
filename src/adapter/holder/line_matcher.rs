use regex::{Regex, RegexBuilder};

use super::pattern_error::PatternError;
use crate::domain::process::{LogPattern, PatternKind, PatternText};

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
    /// The pattern `text` matched as `kind`, checked by compiling it, so a
    /// pattern refused here is never stored or sent.
    ///
    /// # Errors
    /// Returns [`PatternError::Malformed`] for blank or overlong text and
    /// [`PatternError::InvalidRegex`] for a regex that does not compile.
    pub fn checked(text: &str, kind: PatternKind) -> Result<LogPattern, PatternError> {
        let pattern = pattern_of(text, kind)?;
        Self::compile(&pattern)?;
        Ok(pattern)
    }

    /// The matcher for `text` matched as `kind`, for a read that only
    /// filters with it.
    ///
    /// # Errors
    /// Returns the [`PatternError`] [`Self::checked`] does.
    pub fn of_text(text: &str, kind: PatternKind) -> Result<Self, PatternError> {
        Self::compile(&pattern_of(text, kind)?)
    }

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

/// The pattern `text` matched as `kind`, its text checked but not compiled.
///
/// # Errors
/// Returns [`PatternError::Malformed`] for blank or overlong text.
fn pattern_of(text: &str, kind: PatternKind) -> Result<LogPattern, PatternError> {
    let text = PatternText::try_new(text).map_err(|_| PatternError::Malformed(text.to_string()))?;
    Ok(LogPattern::builder().text(text).kind(kind).build())
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

    /// A checked pattern is trimmed and compiled; blank text and a regex
    /// that does not compile are refused before anything stores them.
    #[test]
    fn a_checked_pattern_is_trimmed_and_compiled() {
        assert_eq!(
            LineMatcher::checked(" ready ", PatternKind::Substring).unwrap(),
            pattern("ready", PatternKind::Substring)
        );
        assert_eq!(
            LineMatcher::checked("  ", PatternKind::Substring),
            Err(PatternError::Malformed("  ".to_string()))
        );
        assert!(matches!(
            LineMatcher::checked("(unclosed", PatternKind::Regex),
            Err(PatternError::InvalidRegex { .. })
        ));
    }
}
