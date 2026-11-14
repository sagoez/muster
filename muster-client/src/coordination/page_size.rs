use nutype::nutype;

/// Entries a listing returns when the caller does not ask for a size.
pub const DEFAULT_LIST_LIMIT: usize = 50;
/// Hard cap on a listing's page.
pub const MAX_LIST_LIMIT: usize = 200;

/// How many entries one page of a listing holds, from 1 to
/// [`MAX_LIST_LIMIT`], so a listing never answers more than a reader can take.
#[nutype(
    validate(greater_or_equal = 1, less_or_equal = MAX_LIST_LIMIT),
    default = DEFAULT_LIST_LIMIT,
    derive(Debug, Clone, Copy, PartialEq, Eq, Display, FromStr, Default)
)]
pub struct PageSize(usize);

#[cfg(test)]
mod tests {
    use super::*;

    /// A size defaults, and zero or one past the cap is refused, never cut.
    #[test]
    fn a_page_size_is_bounded() {
        assert_eq!(PageSize::default().into_inner(), DEFAULT_LIST_LIMIT);
        assert!(PageSize::try_new(MAX_LIST_LIMIT).is_ok());
        assert!(PageSize::try_new(0).is_err());
        assert!(PageSize::try_new(MAX_LIST_LIMIT + 1).is_err());
    }
}
