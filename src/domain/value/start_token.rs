use nutype::nutype;

/// A process's creation marker: paired with its
/// [`Pid`](crate::domain::value::Pid), it tells a live process from a later
/// one that reused the id.
#[nutype(
    validate(greater = 0),
    derive(
        Debug,
        Clone,
        Copy,
        PartialEq,
        Eq,
        PartialOrd,
        Ord,
        Hash,
        Display,
        Serialize,
        Deserialize
    )
)]
pub struct StartToken(u64);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_zero() {
        assert!(StartToken::try_new(0).is_err());
    }
}
