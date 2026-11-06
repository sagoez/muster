use nutype::nutype;

/// Prefix of the variables muster itself exports to every process.
const RESERVED_PREFIX: &str = "MUSTER_";
/// Separates a name from its value in an environment entry.
const ENTRY_SEPARATOR: char = '=';
/// Terminates a C string, so no environment entry may contain it.
const NUL: char = '\0';

/// Most characters a new variable name holds, since agents read every name.
pub const ENV_NAME_MAX_CHARS: usize = 128;

/// Whether `name` can be exported to a child without clashing with muster's own.
fn is_env_name(name: &str) -> bool {
    !name.is_empty() && !name.contains([ENTRY_SEPARATOR, NUL]) && !name.starts_with(RESERVED_PREFIX)
}

/// The name of an environment variable a process is given: non-empty, free
/// of `=` and NUL, and outside the `MUSTER_` namespace muster sets itself.
/// [`ENV_NAME_MAX_CHARS`] bounds a new one, never a stored one.
#[nutype(
    validate(predicate = is_env_name),
    derive(
        Debug,
        Clone,
        PartialEq,
        Eq,
        Hash,
        PartialOrd,
        Ord,
        AsRef,
        Display,
        FromStr,
        Serialize,
        Deserialize
    )
)]
pub struct EnvName(String);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_an_ordinary_name() {
        assert_eq!(EnvName::try_new("PORT").unwrap().as_ref(), "PORT");
        assert!(EnvName::try_new("muster_lowercase").is_ok());
    }

    #[test]
    fn refuses_empty_separator_nul_and_reserved_names() {
        for refused in ["", "A=B", "A\0B", "MUSTER_PROJECT"] {
            assert!(EnvName::try_new(refused).is_err(), "{refused:?}");
        }
    }
}
