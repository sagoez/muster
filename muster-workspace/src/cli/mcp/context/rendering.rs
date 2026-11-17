//! How shared text shows in the session context. Each character renders on
//! its own, escaped, so a rendering is the sum of its characters' and a fit
//! to a bound takes one pass, never cutting inside an escape. Text cannot
//! close the block because its fence is random for each run and angle
//! brackets are escaped; no list of lookalikes is kept.

use muster_domain::value::steers_the_terminal;
use unicode_properties::{GeneralCategory, UnicodeGeneralCategory};

/// Opens and closes a quoted string.
const QUOTE_MARK: &str = "\"";
/// A quote mark, which a JSON string escapes.
const QUOTE: char = '"';
/// A backslash, which a JSON string escapes.
const BACKSLASH: char = '\\';
/// A tab, the one control character that reaches a JSON string unescaped.
const TAB: char = '\t';
/// A quote mark inside a JSON string.
const ESCAPED_QUOTE: &str = "\\\"";
/// A backslash inside a JSON string.
const ESCAPED_BACKSLASH: &str = "\\\\";
/// A tab inside a JSON string.
const ESCAPED_TAB: &str = "\\t";
/// The one space a quoted string shows as itself.
const SPACE: char = ' ';
/// Letters that show as blank space. A quoted string shows each as its
/// escape, so none can hide text.
const BLANK_LETTERS: [char; 5] = ['\u{115f}', '\u{1160}', '\u{3164}', '\u{ffa0}', '\u{2800}'];
/// The angle brackets, which shared text shows as escapes such as `\u{3c}`,
/// so no text can open or close a tag.
const ANGLES: [char; 2] = ['<', '>'];
/// The most combining marks one base character keeps as themselves: enough
/// for any script's stacked vowels and tones, while a longer run, which can
/// smear over other text or carry hidden data, shows the rest as escapes.
const MARKS_PER_BASE: usize = 4;
/// Opens each line of a note's body.
pub const BODY_QUOTE: &str = "    > ";
/// Breaks a body's lines.
const LINE_BREAK: char = '\n';
/// A cut falls back to a line break only when the line keeps more than one
/// in this many of the room, so a short first line never hides the rest.
const LINE_CUT_SHARE: usize = 2;

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
        let mut marks = None;
        for (at, character) in raw.char_indices() {
            let before = out.len();
            let stray = if is_mark(character) {
                marks = marks.map(|count: usize| count + 1);
                marks.is_none_or(|count| count > MARKS_PER_BASE)
            } else {
                marks = is_base(character).then_some(0);
                false
            };
            self.push_piece(&mut out, character, stray);
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

    /// Writes how `character` renders onto `out`, allocating nothing; a
    /// `stray` mark shows as its escape.
    fn push_piece(self, out: &mut String, character: char, stray: bool) {
        match self {
            Self::Quoted => push_shared(out, character, true, stray),
            Self::Body if character == LINE_BREAK => {
                out.push(LINE_BREAK);
                out.push_str(BODY_QUOTE);
            },
            Self::Body => push_shared(out, character, false, stray),
        }
    }
}

/// Whether `character` is a combining mark, drawn on the character before it.
fn is_mark(character: char) -> bool {
    matches!(
        character.general_category(),
        GeneralCategory::NonspacingMark | GeneralCategory::EnclosingMark
    )
}

/// Whether `character` shows as itself and can carry the marks after it:
/// not blank and escaped by no rendering.
fn is_base(character: char) -> bool {
    !character.is_whitespace() && !steers_the_terminal(character) && !ANGLES.contains(&character)
}

/// Writes `character` onto `out` as shared text shows it: an angle bracket
/// or a `stray` mark as its unicode escape, a character that would steer a
/// terminal escaped as text, and, `in_string`, a character that shows as
/// blank as its unicode escape, and each character so written escaped for
/// a JSON string.
fn push_shared(out: &mut String, character: char, in_string: bool, stray: bool) {
    let shown = |out: &mut String, shown: char| {
        if in_string {
            push_in_string(out, shown);
        } else {
            out.push(shown);
        }
    };
    if ANGLES.contains(&character) || stray || (in_string && blank_in_a_string(character)) {
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

/// Whether `character` shows as blank space inside a quoted string, where
/// it could hide where the string ends: a space other than the plain one,
/// or a blank letter. What shows as nothing is a format or control
/// character, which [`steers_the_terminal`] owns, or a mark, which
/// [`Rendering::fitted`] judges by what it follows.
fn blank_in_a_string(character: char) -> bool {
    BLANK_LETTERS.contains(&character)
        || (character.general_category() == GeneralCategory::SpaceSeparator && character != SPACE)
}

/// Writes `character`, which steers no terminal, onto `out` as a JSON
/// string holds it, as `serde_json` writes it.
fn push_in_string(out: &mut String, character: char) {
    match character {
        QUOTE => out.push_str(ESCAPED_QUOTE),
        BACKSLASH => out.push_str(ESCAPED_BACKSLASH),
        TAB => out.push_str(ESCAPED_TAB),
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

    /// Inside a quoted string, characters that show as nothing or blank show
    /// as escapes: a zero-width space, a blank letter, an ideographic space
    /// and a mark with nothing before it to sit on; a plain space, an
    /// accented letter and an emoji show as themselves.
    #[test]
    fn a_quoted_string_shows_what_could_hide_as_escapes() {
        let hidden = "\u{200b}\u{3164}\u{3000}\u{e0100}";
        let shown = whole(Rendering::Quoted, hidden);
        assert_eq!(
            shown.matches("u{").count(),
            hidden.chars().count(),
            "{shown}"
        );
        assert_eq!(whole(Rendering::Quoted, "a é 😀"), "\"a é 😀\"");
    }

    /// Script text keeps its marks: Devanagari, Thai, a decomposed accent
    /// and an emoji's presentation selector show as themselves, in a quoted
    /// string and a body alike. A mark with no base, after a space or at the
    /// start, shows as its escape, and so does every mark past
    /// [`MARKS_PER_BASE`] on one base, so no run of them smears over other
    /// text or carries data unseen.
    #[test]
    fn marks_show_on_their_base_and_stray_ones_as_escapes() {
        for script in [
            "नमस्ते हिन्दी",
            "ที่นี่",
            "Cafe\u{301} Vie\u{323}\u{302}t",
            "❤\u{fe0f}",
        ] {
            assert_eq!(whole(Rendering::Quoted, script), format!("\"{script}\""));
            assert_eq!(
                whole(Rendering::Body, script),
                format!("{BODY_QUOTE}{script}")
            );
        }
        assert_eq!(
            whole(Rendering::Body, "\u{301}a \u{fe0f}"),
            format!("{BODY_QUOTE}\\u{{301}}a \\u{{fe0f}}")
        );
        let smuggled = format!("a{}", "\u{fe00}".repeat(MARKS_PER_BASE + 6));
        let shown = whole(Rendering::Body, &smuggled);
        assert_eq!(shown.matches("u{fe00}").count(), 6, "{shown}");
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

    /// No angle bracket shows as itself, so text cannot write a tag.
    #[test]
    fn no_angle_bracket_shows_as_itself() {
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
