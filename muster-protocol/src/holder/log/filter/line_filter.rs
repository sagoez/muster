use super::line_matcher::LineMatcher;

/// Which lines of a log a read keeps.
#[derive(Debug, Clone, Default)]
pub enum LineFilter {
    /// Every line.
    #[default]
    All,
    /// Lines the matcher matches.
    Grep(LineMatcher),
    /// Error lines, each with the continuation lines that follow it, so a
    /// stack trace arrives whole.
    ErrorsOnly,
}
