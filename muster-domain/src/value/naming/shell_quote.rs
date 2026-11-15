/// The quote a bare word is wrapped in, so it reads as a name too.
const QUOTE: char = '\'';

/// `text` quoted so that pasting it into a shell gives it back, even with
/// spaces or an apostrophe, and always quoted so a bare word reads as a name
/// too.
#[must_use]
pub fn shell_quoted(text: &str) -> String {
    let quoted = shlex::try_quote(text)
        .map(|quoted| quoted.into_owned())
        .unwrap_or_else(|_| text.to_string());
    if quoted == text {
        format!("{QUOTE}{quoted}{QUOTE}")
    } else {
        quoted
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Quoted text pastes into a shell as itself, apostrophe and all, and a
    /// bare word is quoted too.
    #[test]
    fn quoted_text_round_trips() {
        for raw in ["api", "dev server", "it's", "my shop"] {
            assert_eq!(shlex::split(&shell_quoted(raw)).unwrap(), vec![raw]);
        }
        assert_eq!(shell_quoted("api"), "'api'");
    }
}
