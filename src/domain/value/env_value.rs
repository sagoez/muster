use nutype::nutype;

/// Terminates a C string, so no environment value may contain it.
const NUL: char = '\0';

/// Most characters a variable value holds: at four bytes a character still
/// 32 KiB, well under the kernel's single-string limit at launch.
pub const ENV_VALUE_MAX_CHARS: usize = 8_192;

/// Whether `value` can be passed to a child's environment.
fn is_env_value(value: &str) -> bool {
    !value.contains(NUL)
}

/// The value of an environment variable a process is given: any text without
/// NUL, the empty string included, at most [`ENV_VALUE_MAX_CHARS`] long.
#[nutype(
    validate(len_char_max = ENV_VALUE_MAX_CHARS, predicate = is_env_value),
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

    /// A value up to the maximum is kept; one past it is refused as too long.
    #[test]
    fn a_value_is_bounded() {
        assert!(EnvValue::try_new("v".repeat(ENV_VALUE_MAX_CHARS)).is_ok());
        assert_eq!(
            EnvValue::try_new("v".repeat(ENV_VALUE_MAX_CHARS + 1)),
            Err(EnvValueError::LenCharMaxViolated)
        );
    }
}
