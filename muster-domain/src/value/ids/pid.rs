use nutype::nutype;

/// The pid of init. It and every lower pid are refused: zero names the
/// caller's process group to a signal and init's group, negated, every
/// process, so neither may ever be signalled.
const INIT_PID: u32 = 1;

/// An operating-system process id above init's, safe to signal.
#[nutype(
    validate(greater = INIT_PID),
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
pub struct Pid(u32);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_zero_and_init() {
        assert!(Pid::try_new(0).is_err());
        assert!(Pid::try_new(INIT_PID).is_err());
        assert_eq!(Pid::try_new(4242).unwrap().into_inner(), 4242);
    }
}
