use nutype::nutype;

/// An operating-system process id; zero names no process.
#[nutype(
    validate(greater = 0),
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
pub struct Pid(u32);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_zero() {
        assert!(Pid::try_new(0).is_err());
        assert_eq!(Pid::try_new(4242).unwrap().into_inner(), 4242);
    }
}
