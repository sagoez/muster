use std::fmt;

use nutype::nutype;

/// The signal that ended a process, by its POSIX name in lowercase (`sigterm`,
/// `sigkill`), the token the wire carries whatever the locale. A signal with no
/// POSIX name is `signal-<number>`. It reads as `SIGTERM` to a person.
#[nutype(
    sanitize(trim, lowercase),
    validate(not_empty),
    derive(Debug, Clone, PartialEq, Eq, AsRef, Serialize, Deserialize)
)]
pub struct SignalName(String);

impl fmt::Display for SignalName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.as_ref().to_uppercase())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The wire token is the lowercase POSIX name, and a person reads the
    /// usual upper-case name.
    #[test]
    fn a_signal_is_a_lowercase_token_read_in_upper_case() {
        let name = SignalName::try_new("SIGTERM").unwrap();

        assert_eq!(name.as_ref(), "sigterm");
        assert_eq!(serde_json::to_value(&name).unwrap(), "sigterm");
        assert_eq!(name.to_string(), "SIGTERM");
    }
}
