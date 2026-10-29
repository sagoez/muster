use nutype::nutype;

/// The fewest rows a PTY may have.
pub const MIN_ROWS: u16 = 1;
/// The most rows a PTY may have, bounding the screen a holder keeps.
pub const MAX_ROWS: u16 = 1000;

/// A count of terminal rows, within [`MIN_ROWS`] to [`MAX_ROWS`].
#[nutype(
    validate(greater_or_equal = MIN_ROWS, less_or_equal = MAX_ROWS),
    derive(
        Debug,
        Clone,
        Copy,
        PartialEq,
        Eq,
        Hash,
        Display,
        Serialize,
        Deserialize
    )
)]
pub struct Rows(u16);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_a_count_outside_the_bounds() {
        assert!(Rows::try_new(MIN_ROWS - 1).is_err());
        assert!(Rows::try_new(MAX_ROWS + 1).is_err());
        assert_eq!(Rows::try_new(24).unwrap().into_inner(), 24);
    }
}
