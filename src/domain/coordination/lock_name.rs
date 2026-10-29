use nutype::nutype;

/// The longest lock name, in characters.
const LOCK_NAME_MAX_CHARS: usize = 64;
/// Punctuation a lock name may hold besides lowercase letters and digits.
const LOCK_NAME_PUNCTUATION: &[char] = &['.', '_', '-'];

/// Whether `name` holds only lowercase ASCII letters, digits and `._-`.
fn is_lock_name(name: &str) -> bool {
    name.chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || LOCK_NAME_PUNCTUATION.contains(&c))
}

/// The name of a project's lock, such as `migrations` or `db-reset`: trimmed,
/// 1 to 64 characters of `[a-z0-9._-]`.
#[nutype(
    sanitize(trim),
    validate(not_empty, len_char_max = LOCK_NAME_MAX_CHARS, predicate = is_lock_name),
    derive(Debug, Clone, PartialEq, Eq, Hash, AsRef, Display)
)]
pub struct LockName(String);

#[cfg(test)]
mod tests {
    use super::*;

    /// A name is trimmed lowercase `[a-z0-9._-]`, 1 to 64 characters.
    #[test]
    fn a_lock_name_is_validated() {
        assert_eq!(
            LockName::try_new("  db-reset ").unwrap().as_ref(),
            "db-reset"
        );
        assert!(LockName::try_new("migrations.v2_a").is_ok());
        assert!(LockName::try_new("").is_err());
        assert!(LockName::try_new("DB").is_err());
        assert!(LockName::try_new("db reset").is_err());
        assert!(LockName::try_new("a".repeat(LOCK_NAME_MAX_CHARS)).is_ok());
        assert!(LockName::try_new("a".repeat(LOCK_NAME_MAX_CHARS + 1)).is_err());
    }
}
