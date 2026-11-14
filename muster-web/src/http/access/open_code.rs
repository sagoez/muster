use std::{
    fmt::{self, Debug, Formatter},
    io,
};

use super::token::ViewToken;

/// What a code prints as in debug output.
const REDACTED: &str = "[REDACTED]";

/// The one-time secret the browser is opened with, as strong as the token
/// and drawn the same way, so the token never reaches a process's
/// arguments. Never printed in debug output.
pub struct OpenCode(ViewToken);

impl OpenCode {
    /// A fresh code from the kernel's random source.
    ///
    /// # Errors
    /// Returns the failure to open or read the random source.
    pub fn generate() -> io::Result<Self> {
        ViewToken::generate().map(Self)
    }

    /// The code's text, for the address the browser is opened at.
    pub fn secret(&self) -> &str {
        self.0.secret()
    }

    /// Whether `candidate` is this code, compared in constant time.
    pub fn matches(&self, candidate: &str) -> bool {
        self.0.matches(candidate)
    }
}

impl Debug for OpenCode {
    /// Prints [`REDACTED`], never the code.
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(REDACTED)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A code is 64 hex characters, fresh each time, matches only itself and
    /// is redacted in debug output.
    #[test]
    fn codes_are_fresh_and_redacted() {
        let first = OpenCode::generate().unwrap();
        let second = OpenCode::generate().unwrap();

        assert_eq!(first.secret().len(), 64);
        assert_ne!(first.secret(), second.secret());
        assert!(first.matches(first.secret()));
        assert!(!first.matches(second.secret()));
        assert_eq!(format!("{first:?}"), "[REDACTED]");
    }
}
