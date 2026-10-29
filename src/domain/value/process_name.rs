use nutype::nutype;
use unicode_normalization::UnicodeNormalization;

use super::steers_the_terminal;

/// Whether `name` holds no control character, tab included, and no format
/// character such as a bidi override or a zero-width space, so printing it
/// can neither drive a terminal nor hide or reorder the text around it.
fn is_terminal_safe(name: &str) -> bool {
    !name
        .chars()
        .any(|character| character.is_control() || steers_the_terminal(character))
}

/// A process's display name: trimmed, non-empty and free of control and
/// format characters.
#[nutype(
    sanitize(trim),
    validate(not_empty, predicate = is_terminal_safe),
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
pub struct ProcessName(String);

impl ProcessName {
    /// The name in Unicode normal form C: two names with the same key read
    /// the same, however their accents were encoded.
    pub fn key(&self) -> String {
        self.as_ref().nfc().collect()
    }

    /// The name quoted so that pasting it into a shell gives the name back,
    /// even with spaces or an apostrophe, and always quoted so a bare word
    /// reads as a name too.
    pub fn shell_quoted(&self) -> String {
        let quoted = shlex::try_quote(self.as_ref())
            .map(|quoted| quoted.into_owned())
            .unwrap_or_else(|_| self.to_string());
        if quoted == self.as_ref() {
            format!("'{quoted}'")
        } else {
            quoted
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A name keys the same however its accent was encoded.
    #[test]
    fn names_equal_in_normal_form_c_share_a_key() {
        let composed = ProcessName::try_new("caf\u{e9}").unwrap();
        let decomposed = ProcessName::try_new("cafe\u{301}").unwrap();

        assert_ne!(composed, decomposed);
        assert_eq!(composed.key(), decomposed.key());
    }

    /// A quoted name pastes into a shell as the name, apostrophe and all.
    #[test]
    fn a_shell_quoted_name_round_trips() {
        for raw in ["api", "dev server", "it's"] {
            let name = ProcessName::try_new(raw).unwrap();
            assert_eq!(shlex::split(&name.shell_quoted()).unwrap(), vec![raw]);
        }
        assert_eq!(ProcessName::try_new("api").unwrap().shell_quoted(), "'api'");
    }

    #[test]
    fn trims_surrounding_whitespace() {
        let name = ProcessName::try_new("  Claude Code  ").unwrap();
        assert_eq!(name.as_ref(), "Claude Code");
    }

    #[test]
    fn rejects_empty_and_whitespace_only() {
        assert!(ProcessName::try_new("").is_err());
        assert!(ProcessName::try_new("   ").is_err());
    }

    /// A name carrying an escape, a bell or an inner line break is refused.
    #[test]
    fn rejects_control_characters() {
        for refused in ["\u{1b}[2Japi", "a\u{7}b", "web\nworker", "x\u{9b}"] {
            assert!(ProcessName::try_new(refused).is_err(), "{refused:?}");
        }
    }

    /// A name carrying a bidi override, an isolate or a zero-width character
    /// is refused, so it cannot reverse or hide the text after it.
    #[test]
    fn rejects_format_characters() {
        for refused in ["api\u{202e}gpj", "a\u{2066}b", "we\u{200b}b", "\u{feff}api"] {
            assert!(ProcessName::try_new(refused).is_err(), "{refused:?}");
        }
        assert!(ProcessName::try_new("caf\u{e9} server").is_ok());
    }
}
