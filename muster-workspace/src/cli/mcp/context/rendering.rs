//! How shared text shows in the session context. Each character renders on
//! its own, escaped and with angle brackets swapped for lookalikes, so a
//! rendering is the sum of its characters' and a fit to a bound takes one
//! pass, never cutting inside an escape.

use std::fmt::Write;

use muster_domain::value::steers_the_terminal;

/// Opens and closes a quoted string.
const QUOTE_MARK: &str = "\"";
/// A quote mark inside a JSON string.
const ESCAPED_QUOTE: &str = "\\\"";
/// A backslash inside a JSON string.
const ESCAPED_BACKSLASH: &str = "\\\\";
/// A line feed inside a JSON string.
const ESCAPED_NEWLINE: &str = "\\n";
/// A tab inside a JSON string.
const ESCAPED_TAB: &str = "\\t";
/// A carriage return inside a JSON string.
const ESCAPED_RETURN: &str = "\\r";
/// A backspace inside a JSON string.
const ESCAPED_BACKSPACE: &str = "\\b";
/// A form feed inside a JSON string.
const ESCAPED_FORM_FEED: &str = "\\f";
/// Opens any other control character inside a JSON string, before its four
/// hex digits.
const UNICODE_ESCAPE: &str = "\\u";
/// The first character a JSON string holds as itself.
const FIRST_PRINTABLE: u32 = 0x20;
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
        let mut out = String::with_capacity(budget.min(raw.len()) + close.len());
        out.push_str(self.open());
        let mut kept = (0, out.len());
        let mut line = None;
        for (at, character) in raw.char_indices() {
            let before = out.len();
            self.push_piece(&mut out, character);
            if out.len() > budget {
                let (end, length) = match line {
                    Some((at, length)) if length * LINE_CUT_SHARE > allowance => (at, length),
                    _ => kept,
                };
                out.truncate(length);
                out.push_str(close);
                return (out, raw.len() - end);
            }
            if character == LINE_BREAK && at > 0 && before + reserve <= budget {
                line = Some((at, before));
            }
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

    /// Writes how `character` renders onto `out`, allocating nothing.
    fn push_piece(self, out: &mut String, character: char) {
        match self {
            Self::Plain => out.push(character),
            Self::Quoted => push_shared(out, character, true),
            Self::Body if character == LINE_BREAK => {
                out.push(LINE_BREAK);
                out.push_str(BODY_QUOTE);
            },
            Self::Body => push_shared(out, character, false),
        }
    }
}

/// Writes `character` onto `out` as shared text shows it: escaped as text
/// when it would steer a terminal, an angle bracket as its lookalike, and,
/// `in_string`, each character so written escaped for a JSON string.
fn push_shared(out: &mut String, character: char, in_string: bool) {
    let shown = |out: &mut String, shown: char| {
        if in_string {
            push_in_string(out, shown);
        } else {
            out.push(shown);
        }
    };
    if character == OPEN_ANGLE.0 {
        shown(out, OPEN_ANGLE.1);
    } else if character == CLOSE_ANGLE.0 {
        shown(out, CLOSE_ANGLE.1);
    } else if steers_the_terminal(character) {
        character
            .escape_default()
            .for_each(|escaped| shown(out, escaped));
    } else {
        shown(out, character);
    }
}

/// Writes `character` onto `out` as a JSON string holds it, as
/// `serde_json` writes it.
fn push_in_string(out: &mut String, character: char) {
    match character {
        '"' => out.push_str(ESCAPED_QUOTE),
        '\\' => out.push_str(ESCAPED_BACKSLASH),
        '\n' => out.push_str(ESCAPED_NEWLINE),
        '\t' => out.push_str(ESCAPED_TAB),
        '\r' => out.push_str(ESCAPED_RETURN),
        '\u{8}' => out.push_str(ESCAPED_BACKSPACE),
        '\u{c}' => out.push_str(ESCAPED_FORM_FEED),
        control if u32::from(control) < FIRST_PRINTABLE => {
            let _ = write!(out, "{UNICODE_ESCAPE}{:04x}", u32::from(control));
        },
        other => out.push(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A character that steers a terminal.
    const ESCAPE: char = '\u{1b}';
    /// How the body shows it.
    const ESCAPED: &str = "\\u{1b}";

    /// Every character escapes for a JSON string exactly as `serde_json`
    /// escapes it, written straight into the one buffer.
    #[test]
    fn a_string_escapes_as_json_does() {
        let samples =
            (0..=0x7F_u32)
                .filter_map(char::from_u32)
                .chain(['\u{e9}', '\u{2028}', '\u{1f600}']);
        for character in samples {
            let mut out = String::new();
            push_in_string(&mut out, character);
            let json = serde_json::to_string(&character.to_string()).unwrap();
            assert_eq!(out, json[1..json.len() - 1], "{:?}", character);
        }
    }

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
