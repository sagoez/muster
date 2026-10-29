use nutype::nutype;

/// The fewest columns a PTY may have: a wide character needs two.
pub const MIN_COLS: u16 = 2;
/// The most columns a PTY may have, bounding the screen a holder keeps.
pub const MAX_COLS: u16 = 2000;

/// A count of terminal columns, within [`MIN_COLS`] to [`MAX_COLS`].
#[nutype(
    validate(greater_or_equal = MIN_COLS, less_or_equal = MAX_COLS),
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
pub struct Cols(u16);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_a_count_outside_the_bounds() {
        assert!(Cols::try_new(MIN_COLS - 1).is_err());
        assert!(Cols::try_new(MAX_COLS + 1).is_err());
        assert_eq!(Cols::try_new(80).unwrap().into_inner(), 80);
    }
}
