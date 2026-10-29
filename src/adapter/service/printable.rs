/// What stands in for each control character a stored string carried.
const CONTROL_REPLACEMENT: char = ' ';

/// `text` with every control character, escape and newline included, replaced
/// by a space, so a string a process or a config produced cannot drive the
/// terminal of whoever later reads it.
pub fn printable(text: &str) -> String {
    text.chars()
        .map(|character| {
            if character.is_control() {
                CONTROL_REPLACEMENT
            } else {
                character
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_characters_are_replaced_and_text_is_kept() {
        assert_eq!(
            printable("crashed: \x1b[31mred\x07\nnext\u{9b}2J café"),
            "crashed:  [31mred  next 2J café"
        );
    }
}
