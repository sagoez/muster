use std::{
    fmt::{self, Debug, Formatter, Write},
    fs::File,
    hint,
    io::{self, Read},
};

/// The kernel's random source, the same on Linux and macOS.
const RANDOM_SOURCE: &str = "/dev/urandom";
/// How many random bytes a token holds: 256 bits.
const TOKEN_BYTES: usize = 32;
/// What a token prints as in debug output.
const REDACTED: &str = "[REDACTED]";

/// The secret a browser must present to the view: 256 random bits as
/// lowercase hex. Never printed in debug output.
#[derive(Clone, PartialEq, Eq)]
pub struct ViewToken(String);

impl ViewToken {
    /// A fresh token from the kernel's random source.
    ///
    /// # Errors
    /// Returns the failure to open or read the random source.
    pub fn generate() -> io::Result<Self> {
        let mut bytes = [0; TOKEN_BYTES];
        File::open(RANDOM_SOURCE)?.read_exact(&mut bytes)?;
        let mut hex = String::with_capacity(TOKEN_BYTES * 2);
        for byte in bytes {
            let _ = write!(hex, "{byte:02x}");
        }
        Ok(Self(hex))
    }

    /// The token's text, for the URL and the cookie the view hands out.
    pub fn secret(&self) -> &str {
        &self.0
    }

    /// Whether `candidate` is this token, compared in constant time so the
    /// time an answer takes never tells how much of a guess was right.
    pub fn matches(&self, candidate: &str) -> bool {
        let expected = self.0.as_bytes();
        let candidate = candidate.as_bytes();
        if expected.len() != candidate.len() {
            return false;
        }
        let difference = expected
            .iter()
            .zip(candidate)
            .fold(0, |difference, (left, right)| difference | (left ^ right));
        hint::black_box(difference) == 0
    }
}

impl Debug for ViewToken {
    /// Prints [`REDACTED`], never the token.
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(REDACTED)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A token is 64 lowercase hex characters, fresh each time, and only
    /// itself matches it.
    #[test]
    fn tokens_are_fresh_hex_and_match_only_themselves() {
        let first = ViewToken::generate().unwrap();
        let second = ViewToken::generate().unwrap();

        assert_eq!(first.secret().len(), 64);
        assert!(
            first
                .secret()
                .chars()
                .all(|character| matches!(character, '0'..='9' | 'a'..='f')),
            "{}",
            first.secret()
        );
        assert_ne!(first, second);
        assert!(first.matches(first.secret()));
        assert!(!first.matches(second.secret()));
        assert!(!first.matches(&first.secret()[1..]));
        assert!(!first.matches(""));
    }

    /// Debug output never shows the token.
    #[test]
    fn debug_is_redacted() {
        let token = ViewToken::generate().unwrap();

        let printed = format!("{token:?}");

        assert_eq!(printed, "[REDACTED]");
        assert!(!printed.contains(token.secret()));
    }
}
