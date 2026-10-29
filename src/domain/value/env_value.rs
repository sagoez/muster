use nutype::nutype;

/// Terminates a C string, so no environment value may contain it.
const NUL: char = '\0';

/// Whether `value` can be passed to a child's environment.
fn is_env_value(value: &str) -> bool {
    !value.contains(NUL)
}

/// The value of an environment variable a process is given: any text without
/// NUL, the empty string included.
#[nutype(
    validate(predicate = is_env_value),
    derive(
        Debug,
        Clone,
        PartialEq,
        Eq,
        Hash,
        AsRef,
        Display,
        FromStr,
        Serialize,
        Deserialize
    )
)]
pub struct EnvValue(String);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_any_text_without_nul() {
        assert!(EnvValue::try_new("").is_ok());
        assert!(EnvValue::try_new("a=b c").is_ok());
        assert!(EnvValue::try_new("a\0b").is_err());
    }
}
