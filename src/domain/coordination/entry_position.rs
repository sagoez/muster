use nutype::nutype;

/// Where a note or key-value entry stands in the order its project's entries
/// were first written. A page resumes after one, so deleting the entry a
/// page ended on never breaks the listing. Opaque to whoever pages.
#[nutype(
    validate(greater_or_equal = 0),
    derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Display, FromStr)
)]
pub struct EntryPosition(i64);

#[cfg(test)]
mod tests {
    use super::*;

    /// A position round-trips through its text; a negative or non-numeric
    /// one is refused.
    #[test]
    fn a_position_round_trips_through_its_text() {
        let position = EntryPosition::try_new(42).unwrap();
        assert_eq!(
            position.to_string().parse::<EntryPosition>().unwrap(),
            position
        );
        assert!("-1".parse::<EntryPosition>().is_err());
        assert!("plan".parse::<EntryPosition>().is_err());
    }
}
