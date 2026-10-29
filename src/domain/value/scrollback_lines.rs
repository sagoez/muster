use nutype::nutype;

/// The most lines of scrollback a process's terminal may keep.
pub const MAX_SCROLLBACK: usize = 100_000;

/// Lines of scrollback each process's terminal keeps, at most
/// [`MAX_SCROLLBACK`].
#[nutype(
    validate(less_or_equal = MAX_SCROLLBACK),
    derive(Debug, Clone, Copy, PartialEq, Eq, Display, FromStr)
)]
pub struct ScrollbackLines(usize);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrollback_is_bounded() {
        assert!(ScrollbackLines::try_new(0).is_ok());
        assert!(ScrollbackLines::try_new(MAX_SCROLLBACK).is_ok());
        assert!(ScrollbackLines::try_new(MAX_SCROLLBACK + 1).is_err());
        assert!("100001".parse::<ScrollbackLines>().is_err());
    }
}
