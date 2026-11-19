use nutype::nutype;

/// The longest lock name, in characters.
pub const LOCK_NAME_MAX_CHARS: usize = 64;
/// Punctuation a lock name may hold besides lowercase letters and digits.
const LOCK_NAME_PUNCTUATION: &[char] = &['.', '_', '-'];
/// The form of a lock name, around its longest length and before the
/// punctuation it may hold.
const FORM_HEAD: &str = "1 to ";
const FORM_TAIL: &str = " of a-z, 0-9, ";
/// Quotes each punctuation mark the form lists.
const QUOTE: char = '\'';
/// Between the punctuation marks the form lists, and before the last.
const LIST_SEPARATOR: &str = ", ";
const LAST_SEPARATOR: &str = " and ";

/// What a lock name may hold, as a refusal states it:
/// `1 to 64 of a-z, 0-9, '.', '_' and '-'`.
pub fn lock_name_form() -> String {
    let quoted: Vec<String> = LOCK_NAME_PUNCTUATION
        .iter()
        .map(|mark| format!("{QUOTE}{mark}{QUOTE}"))
        .collect();
    let punctuation = match quoted.split_last() {
        Some((last, rest)) if !rest.is_empty() => {
            format!("{}{LAST_SEPARATOR}{last}", rest.join(LIST_SEPARATOR))
        },
        Some((last, _)) => last.clone(),
        None => String::new(),
    };
    format!("{FORM_HEAD}{LOCK_NAME_MAX_CHARS}{FORM_TAIL}{punctuation}")
}

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

    /// The form a refusal states reads from the length and punctuation the
    /// check enforces.
    #[test]
    fn the_form_states_the_length_and_punctuation() {
        assert_eq!(lock_name_form(), "1 to 64 of a-z, 0-9, '.', '_' and '-'");
    }
}
