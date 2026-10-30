//! The one place text from a process, the service or stored state is made
//! safe to show on a person's terminal: the service applies it to what it
//! stores, the CLI to what it prints. The rule itself is the domain's
//! [`printable`], which project names are validated against too.

pub use crate::domain::value::printable;

/// Separates the lines [`printable_lines`] keeps.
const LINE_BREAK: char = '\n';

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

    /// A message spanning lines keeps its breaks and loses everything else.
    #[test]
    fn line_breaks_survive_when_asked() {
        assert_eq!(printable_lines("one\n\u{1b}[2Jtwo"), "one\n\\u{1b}[2Jtwo");
    }
}
