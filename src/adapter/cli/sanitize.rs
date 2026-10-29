//! The one place text from a process, the service or stored state is made
//! safe to show on a person's terminal.

/// Separates the lines [`printable_lines`] keeps.
const LINE_BREAK: char = '\n';
/// The one control character left as it is: it only moves to a tab stop.
const TAB: char = '\t';

/// Whether `character` could steer the terminal it is printed on.
fn steers_the_terminal(character: char) -> bool {
    character.is_control() && character != TAB
}

/// `text` with every control character but tab escaped, so a process name,
/// notice, exit reason or title cannot move the cursor, retitle the window or
/// write to the clipboard of the terminal it is shown on.
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

/// `text` made [`printable`] line by line, keeping its line breaks: for a
/// message that spans lines on purpose, such as a failure with a log tail.
pub fn printable_lines(text: &str) -> String {
    text.split(LINE_BREAK)
        .map(printable)
        .collect::<Vec<_>>()
        .join(&LINE_BREAK.to_string())
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

    /// A message spanning lines keeps its breaks and loses everything else.
    #[test]
    fn line_breaks_survive_when_asked() {
        assert_eq!(printable_lines("one\n\u{1b}[2Jtwo"), "one\n\\u{1b}[2Jtwo");
    }
}
