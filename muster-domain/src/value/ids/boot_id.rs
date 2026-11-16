use nutype::nutype;

/// The identity of one boot of the system: a start token counts from boot,
/// so a token recorded in an earlier boot may come round again, and only a
/// boot id that is still the same says a recorded process may still run.
#[nutype(
    sanitize(trim),
    validate(not_empty),
    derive(
        Debug,
        Clone,
        PartialEq,
        Eq,
        Hash,
        AsRef,
        Display,
        Serialize,
        Deserialize
    )
)]
pub struct BootId(String);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_blank_and_trims() {
        assert!(BootId::try_new(" \n").is_err());
        assert_eq!(BootId::try_new("abc\n").unwrap().as_ref(), "abc");
    }
}
