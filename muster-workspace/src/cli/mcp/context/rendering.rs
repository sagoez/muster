//! How shared text shows in the session context. Each character renders on
//! its own, escaped, so a rendering is the sum of its characters' and a fit
//! to a bound takes one pass, never cutting inside an escape.

use muster_domain::value::steers_the_terminal;
use unicode_properties::{GeneralCategory, UnicodeGeneralCategory};

/// Opens and closes a quoted string.
const QUOTE_MARK: &str = "\"";
/// A quote mark inside a JSON string.
const ESCAPED_QUOTE: &str = "\\\"";
/// A backslash inside a JSON string.
const ESCAPED_BACKSLASH: &str = "\\\\";
/// A tab inside a JSON string, the one control character that reaches one
/// unescaped.
const ESCAPED_TAB: &str = "\\t";
/// The one space a quoted string shows as itself.
const SPACE: char = ' ';
/// Characters that look like a quote mark. A quoted string shows each as
/// its escape, so none can seem to end the string.
const QUOTES: [char; 11] = [
    '\u{ff02}', '\u{201c}', '\u{201d}', '\u{201e}', '\u{201f}', '\u{2033}', '\u{02ba}', '\u{3003}',
    '\u{275d}', '\u{275e}', '\u{2036}',
];
/// Letters that show as blank space. A quoted string shows each as its
/// escape, so none can hide text.
const BLANK_LETTERS: [char; 5] = ['\u{115f}', '\u{1160}', '\u{3164}', '\u{ffa0}', '\u{2800}'];
/// Opens each line of a note's body.
pub const BODY_QUOTE: &str = "    > ";
/// Breaks a body's lines.
const LINE_BREAK: char = '\n';
/// A cut falls back to a line break only when the line keeps more than one
/// in this many of the room, so a short first line never hides the rest.
const LINE_CUT_SHARE: usize = 2;
/// Angle brackets and the characters that look like them. Shared text shows
/// each as its escape, such as `\u{3c}`, so no text can open or close a
/// tag, or seem to.
const ANGLES: [char; 18] = [
    '<', '>', '\u{2039}', '\u{203a}', '\u{ff1c}', '\u{ff1e}', '\u{2329}', '\u{232a}', '\u{3008}',
    '\u{3009}', '\u{27e8}', '\u{27e9}', '\u{fe64}', '\u{fe65}', '\u{276e}', '\u{276f}', '\u{1438}',
    '\u{1433}',
];

/// How a piece of shared text is rendered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rendering {
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
            Self::Quoted => QUOTE_MARK,
            Self::Body => BODY_QUOTE,
        }
    }

    /// What closes a rendering.
    fn close(self) -> &'static str {
        match self {
            Self::Quoted => QUOTE_MARK,
            Self::Body => "",
        }
    }

    /// Writes how `character` renders onto `out`, allocating nothing.
    fn push_piece(self, out: &mut String, character: char) {
        match self {
            Self::Quoted => push_shared(out, character, true),
            Self::Body if character == LINE_BREAK => {
                out.push(LINE_BREAK);
                out.push_str(BODY_QUOTE);
            },
            Self::Body => push_shared(out, character, false),
        }
    }
}

/// Writes `character` onto `out` as shared text shows it: an angle bracket
/// or its lookalike as its unicode escape, a character that would steer a
/// terminal escaped as text, and, `in_string`, a quote mark lookalike or a
/// character that shows as nothing or blank as its unicode escape, and each
/// character so written escaped for a JSON string.
fn push_shared(out: &mut String, character: char, in_string: bool) {
    let shown = |out: &mut String, shown: char| {
        if in_string {
            push_in_string(out, shown);
        } else {
            out.push(shown);
        }
    };
    if ANGLES.contains(&character) || (in_string && hides_in_a_string(character)) {
        character
            .escape_unicode()
            .for_each(|escaped| shown(out, escaped));
    } else if steers_the_terminal(character) {
        character
            .escape_default()
            .for_each(|escaped| shown(out, escaped));
    } else {
        shown(out, character);
    }
}

/// Whether `character` could pass for a quote mark, or shows as nothing or
/// as blank space, inside a quoted string: a quote lookalike, a mark that
/// only modifies what precedes it, such as a variation selector, a format
/// character, a space other than the plain one, or a blank letter.
fn hides_in_a_string(character: char) -> bool {
    QUOTES.contains(&character)
        || BLANK_LETTERS.contains(&character)
        || matches!(
            character.general_category(),
            GeneralCategory::NonspacingMark
                | GeneralCategory::EnclosingMark
                | GeneralCategory::Format
        )
        || (character.general_category() == GeneralCategory::SpaceSeparator && character != SPACE)
}

/// Writes `character`, which steers no terminal, onto `out` as a JSON
/// string holds it, as `serde_json` writes it.
fn push_in_string(out: &mut String, character: char) {
    match character {
        '"' => out.push_str(ESCAPED_QUOTE),
        '\\' => out.push_str(ESCAPED_BACKSLASH),
        '\t' => out.push_str(ESCAPED_TAB),
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

    /// Every character that steers no terminal escapes for a JSON string
    /// exactly as `serde_json` escapes it, written straight into the one
    /// buffer.
    #[test]
    fn a_string_escapes_as_json_does() {
        let samples = (0..=0x7F_u32)
            .filter_map(char::from_u32)
            .chain(['\u{e9}', '\u{1f600}'])
            .filter(|character| !steers_the_terminal(*character));
        for character in samples {
            let mut out = String::new();
            push_in_string(&mut out, character);
            let json = serde_json::to_string(&character.to_string()).unwrap();
            assert_eq!(out, json[1..json.len() - 1], "{:?}", character);
        }
    }

    /// Inside a quoted string, quote lookalikes and characters that show as
    /// nothing or blank show as escapes; a plain space, an accented letter
    /// and an emoji show as themselves.
    #[test]
    fn a_quoted_string_shows_what_could_hide_as_escapes() {
        let hidden = "\u{ff02}\u{201c}\u{fe0f}\u{200b}\u{3164}\u{3000}\u{e0100}";
        let shown = whole(Rendering::Quoted, hidden);
        assert_eq!(
            shown.matches("u{").count(),
            hidden.chars().count(),
            "{shown}"
        );
        assert_eq!(whole(Rendering::Quoted, "a é 😀"), "\"a é 😀\"");
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
        assert_eq!(
            whole(Rendering::Quoted, "a \"b\" <c>\u{1b}"),
            "\"a \\\"b\\\" \\\\u{3c}c\\\\u{3e}\\\\u{1b}\""
        );
        assert_eq!(
            whole(Rendering::Body, "one\n<two>"),
            "    > one\n    > \\u{3c}two\\u{3e}"
        );
    }

    /// No angle bracket nor any of its lookalikes shows as itself, so text
    /// cannot write a tag, or one that only looks like one.
    #[test]
    fn no_angle_bracket_or_lookalike_shows_as_itself() {
        let raw: String = ANGLES.iter().collect();
        for rendering in [Rendering::Quoted, Rendering::Body] {
            let shown = whole(rendering, &raw);
            let text = shown.trim_start_matches(rendering.open());
            assert!(
                text.chars().all(|character| !ANGLES.contains(&character)),
                "{shown}"
            );
            assert_eq!(shown.matches("u{").count(), ANGLES.len(), "{shown}");
        }
    }

    /// A fit ends on a character and, when it can, on a line, measures what
    /// it renders, and leaves room for the caller's mark only when it cuts.
    #[test]
    fn a_fit_ends_on_a_character_and_a_line() {
        assert_eq!(
            Rendering::Body.fitted("abc\ndef", 10, 0),
            ("    > abc".to_owned(), 4)
        );
        let paragraph = format!("ab\n{}", "c".repeat(20));
        assert_eq!(
            Rendering::Body.fitted(&paragraph, 27, 0),
            (format!("    > ab\n    > {}", "c".repeat(12)), 8),
            "a short first line does not hide the paragraph after it"
        );
        assert_eq!(
            Rendering::Quoted.fitted("ééé", 4, 0),
            ("\"é\"".to_owned(), 4)
        );
        assert_eq!(
            Rendering::Quoted.fitted("short", 10, 0),
            ("\"short\"".to_owned(), 0)
        );
        assert_eq!(
            Rendering::Quoted.fitted("exactly", 9, 3),
            ("\"exactly\"".to_owned(), 0)
        );
        assert_eq!(
            Rendering::Quoted.fitted("exactly!", 9, 3),
            ("\"exac\"".to_owned(), 4)
        );
        assert_eq!(
            Rendering::Quoted.fitted("abcdef", 6, 0),
            ("\"abcd\"".to_owned(), 2)
        );
        let (shown, _) = Rendering::Body.fitted(&ESCAPE.to_string().repeat(10), 26, 0);
        assert_eq!(shown, format!("{BODY_QUOTE}{}", ESCAPED.repeat(3)));
    }
}
