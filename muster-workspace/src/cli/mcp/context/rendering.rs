//! How shared text shows in the session context. Each character renders on
//! its own, escaped and with angle brackets swapped for lookalikes, so a
//! rendering is the sum of its characters' and a fit to a bound takes one
//! pass, never cutting inside an escape.

use muster_domain::value::printable;

/// Opens and closes a quoted string.
const QUOTE_MARK: &str = "\"";
/// Opens each line of a note's body.
pub const BODY_QUOTE: &str = "    > ";
/// Breaks a body's lines.
const LINE_BREAK: char = '\n';
/// A cut falls back to a line break only when the line keeps more than one
/// in this many of the room, so a short first line never hides the rest.
const LINE_CUT_SHARE: usize = 2;
/// An opening angle bracket and the lookalike shared text shows it as, so
/// no text can open a tag.
const OPEN_ANGLE: (char, char) = ('<', '\u{2039}');
/// A closing angle bracket and its lookalike.
const CLOSE_ANGLE: (char, char) = ('>', '\u{203a}');

/// How a piece of shared text is rendered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rendering {
    /// As written: a name of plain characters only.
    Plain,
    /// As one quoted string, every control character escaped.
    Quoted,
    /// As a note's body, every line quoted with [`BODY_QUOTE`].
    Body,
}

impl Rendering {
    /// The rendering of the longest start of `raw` within `max` bytes, and
    /// how many bytes of `raw` it left off. When all of `raw` does not fit,
    /// the start kept fits within `max` less `reserve`, room for what the
    /// caller adds to say it was cut, and ends on a line when one ends late
    /// enough to keep most of that room, else at the last character that
    /// fits.
    pub fn fitted(self, raw: &str, max: usize, reserve: usize) -> (String, usize) {
        let close = self.close();
        let budget = max.saturating_sub(close.len());
        let allowance = budget.saturating_sub(reserve);
        let mut out = String::from(self.open());
        let mut kept = (0, out.len());
        let mut line = None;
        for (at, character) in raw.char_indices() {
            let piece = self.piece(character);
            if out.len() + piece.len() > budget {
                let (end, length) = match line {
                    Some((at, length)) if length * LINE_CUT_SHARE > allowance => (at, length),
                    _ => kept,
                };
                out.truncate(length);
                out.push_str(close);
                return (out, raw.len() - end);
            }
            if character == LINE_BREAK && at > 0 && out.len() + reserve <= budget {
                line = Some((at, out.len()));
            }
            out.push_str(&piece);
            if out.len() + reserve <= budget {
                kept = (at + character.len_utf8(), out.len());
            }
        }
        out.push_str(close);
        (out, 0)
    }

    /// What opens a rendering.
    fn open(self) -> &'static str {
        match self {
            Self::Plain => "",
            Self::Quoted => QUOTE_MARK,
            Self::Body => BODY_QUOTE,
        }
    }

    /// What closes a rendering.
    fn close(self) -> &'static str {
        match self {
            Self::Quoted => QUOTE_MARK,
            Self::Plain | Self::Body => "",
        }
    }

    /// How `character` renders.
    fn piece(self, character: char) -> String {
        match self {
            Self::Plain => character.to_string(),
            Self::Quoted => {
                let escaped = serde_json::to_string(&shared(character)).unwrap_or_default();
                escaped
                    .strip_prefix(QUOTE_MARK)
                    .and_then(|inner| inner.strip_suffix(QUOTE_MARK))
                    .unwrap_or_default()
                    .to_owned()
            },
            Self::Body if character == LINE_BREAK => format!("{LINE_BREAK}{BODY_QUOTE}"),
            Self::Body => shared(character),
        }
    }
}

/// `character` as shared text shows it: escaped when it would steer a
/// terminal, an angle bracket as its lookalike.
fn shared(character: char) -> String {
    match character {
        open if open == OPEN_ANGLE.0 => OPEN_ANGLE.1.to_string(),
        close if close == CLOSE_ANGLE.0 => CLOSE_ANGLE.1.to_string(),
        other => printable(&other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A character that steers a terminal.
    const ESCAPE: char = '\u{1b}';
    /// How the body shows it.
    const ESCAPED: &str = "\\u{1b}";

    /// All of `raw` as `rendering` shows it.
    fn whole(rendering: Rendering, raw: &str) -> String {
        rendering.fitted(raw, usize::MAX, 0).0
    }

    /// A rendering is the sum of its characters': quoted text escapes once,
    /// for the terminal and then as a string, and its brackets can open no
    /// tag; a body quotes each line.
    #[test]
    fn each_rendering_shows_shared_text_safely() {
        assert_eq!(whole(Rendering::Plain, "claude"), "claude");
        assert_eq!(
            whole(Rendering::Quoted, "a \"b\" <c>\u{1b}"),
            "\"a \\\"b\\\" \u{2039}c\u{203a}\\\\u{1b}\""
        );
        assert_eq!(
            whole(Rendering::Body, "one\n<two>"),
            "    > one\n    > \u{2039}two\u{203a}"
        );
    }

    /// A fit ends on a character and, when it can, on a line, measures what
    /// it renders, and leaves room for the caller's mark only when it cuts.
    #[test]
    fn a_fit_ends_on_a_character_and_a_line() {
        assert_eq!(
            Rendering::Plain.fitted("abc\ndef", 5, 0),
            ("abc".to_owned(), 4)
        );
        let paragraph = format!("ab\n{}", "c".repeat(20));
        assert_eq!(
            Rendering::Plain.fitted(&paragraph, 15, 0),
            (format!("ab\n{}", "c".repeat(12)), 8),
            "a short first line does not hide the paragraph after it"
        );
        assert_eq!(Rendering::Plain.fitted("ééé", 3, 0), ("é".to_owned(), 4));
        assert_eq!(
            Rendering::Plain.fitted("short", 10, 0),
            ("short".to_owned(), 0)
        );
        assert_eq!(
            Rendering::Plain.fitted("exactly", 7, 3),
            ("exactly".to_owned(), 0)
        );
        assert_eq!(
            Rendering::Plain.fitted("exactly!", 7, 3),
            ("exac".to_owned(), 4)
        );
        assert_eq!(
            Rendering::Quoted.fitted("abcdef", 6, 0),
            ("\"abcd\"".to_owned(), 2)
        );
        let (shown, _) = Rendering::Body.fitted(&ESCAPE.to_string().repeat(10), 26, 0);
        assert_eq!(shown, format!("{BODY_QUOTE}{}", ESCAPED.repeat(3)));
    }
}
