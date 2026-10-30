use unicode_properties::{GeneralCategory, UnicodeGeneralCategory};

/// The one control character left as it is: it only moves to a tab stop.
const TAB: char = '\t';

/// Whether `character` could steer the terminal it is printed on, or hide or
/// reorder the text around it: a control character, or a format character
/// such as a bidi override or a zero-width space.
pub fn steers_the_terminal(character: char) -> bool {
    (character.is_control() && character != TAB)
        || character.general_category() == GeneralCategory::Format
}

/// `text` with every control and format character but tab escaped, so a
/// process name, notice, exit reason or title cannot move the cursor,
/// retitle the window, write to the clipboard of the terminal it is shown on,
/// or read as something it is not.
pub fn printable(text: &str) -> String {
    if !text.chars().any(steers_the_terminal) {
        return text.to_string();
    }
    text.chars()
        .map(|character| {
            if steers_the_terminal(character) {
                character.escape_default().to_string()
            } else {
                character.to_string()
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An escape sequence, a carriage return and a bell are shown as text,
    /// never sent to the terminal.
    #[test]
    fn control_characters_are_escaped() {
        assert_eq!(printable("api"), "api");
        assert_eq!(printable("caf\u{e9} server"), "caf\u{e9} server");
        assert_eq!(
            printable("\u{1b}]52;c;aGk=\u{7}x"),
            "\\u{1b}]52;c;aGk=\\u{7}x"
        );
        assert_eq!(printable("a\rb\nc"), "a\\rb\\nc");
        assert_eq!(printable("key\tvalue"), "key\tvalue");
        assert!(!printable("\u{9b}31m").chars().any(char::is_control));
    }

    /// Bidi controls and zero-width characters are shown as escapes, so a
    /// name cannot reverse or hide the text after it.
    #[test]
    fn format_characters_are_escaped() {
        assert_eq!(printable("api\u{202e}gpj.exe"), "api\\u{202e}gpj.exe");
        assert_eq!(printable("a\u{2066}b\u{2069}"), "a\\u{2066}b\\u{2069}");
        assert_eq!(printable("we\u{200b}b"), "we\\u{200b}b");
        assert_eq!(printable("\u{feff}api"), "\\u{feff}api");
    }
}
