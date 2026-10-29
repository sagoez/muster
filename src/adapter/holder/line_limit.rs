use nutype::nutype;

/// The most lines one log read returns.
pub const MAX_LOG_LINES: usize = 2000;
/// Lines a log read returns when the caller names no limit.
const DEFAULT_LOG_LINES: usize = 100;

/// How many lines a log read returns at most: 1 to [`MAX_LOG_LINES`].
#[nutype(
    validate(greater = 0, less_or_equal = MAX_LOG_LINES),
    default = DEFAULT_LOG_LINES,
    derive(Debug, Clone, Copy, PartialEq, Eq, Display, Default, Serialize, Deserialize)
)]
pub struct LineLimit(usize);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_limit_is_bounded_and_defaults_to_a_hundred() {
        assert_eq!(LineLimit::default().into_inner(), DEFAULT_LOG_LINES);
        assert!(LineLimit::try_new(0).is_err());
        assert!(LineLimit::try_new(MAX_LOG_LINES).is_ok());
        assert!(LineLimit::try_new(MAX_LOG_LINES + 1).is_err());
    }
}
