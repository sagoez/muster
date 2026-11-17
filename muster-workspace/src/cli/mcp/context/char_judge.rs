use std::ops::RangeInclusive;

use muster_domain::value::steers_the_terminal;
use unicode_properties::{GeneralCategory, UnicodeEmoji, UnicodeGeneralCategory};

use crate::cli::mcp::context::shown_as::ShownAs;

/// The most combining marks one base character keeps as themselves: enough
/// for fully pointed Hebrew and any script's stacked vowels and tones, while
/// a longer run, which can smear over other text, shows the rest as escapes.
pub const MARKS_PER_BASE: usize = 6;
/// Unicode's Default_Ignorable_Code_Point ranges, as of Unicode 16: what a
/// renderer draws as nothing when it has no use for it. Each shows as its
/// escape, but for [`ZERO_WIDTH_JOINER`] inside an emoji sequence.
const DEFAULT_IGNORABLE: [RangeInclusive<char>; 17] = [
    '\u{ad}'..='\u{ad}',
    '\u{34f}'..='\u{34f}',
    '\u{61c}'..='\u{61c}',
    '\u{115f}'..='\u{1160}',
    '\u{17b4}'..='\u{17b5}',
    '\u{180b}'..='\u{180f}',
    '\u{200b}'..='\u{200f}',
    '\u{202a}'..='\u{202e}',
    '\u{2060}'..='\u{206f}',
    '\u{3164}'..='\u{3164}',
    '\u{fe00}'..='\u{fe0f}',
    '\u{feff}'..='\u{feff}',
    '\u{ffa0}'..='\u{ffa0}',
    '\u{fff0}'..='\u{fff8}',
    '\u{1bca0}'..='\u{1bca3}',
    '\u{1d173}'..='\u{1d17a}',
    '\u{e0000}'..='\u{e0fff}',
];
/// The zero-width joiner, which joins emoji into one glyph.
const ZERO_WIDTH_JOINER: char = '\u{200d}';
/// The angle brackets, which shared text always shows as escapes.
pub const ANGLES: [char; 2] = ['<', '>'];

/// Judges each character of one piece of shared text by what came before
/// it and what comes next: a default-ignorable character shows as its
/// escape unless it is a zero-width joiner between two pictographs, and a
/// mark shows as itself only on a base, at most [`MARKS_PER_BASE`] of them.
/// A private helper of
/// [`Rendering::fitted`](super::rendering::Rendering::fitted), built with
/// [`Default`].
#[derive(Debug, Default)]
pub struct CharJudge {
    /// The marks on the current base, none when nothing before can carry one.
    marks: Option<usize>,
    /// The character before, if any.
    previous: Option<char>,
}

impl CharJudge {
    /// How `character`, followed by `next`, shows.
    pub fn judge(&mut self, character: char, next: Option<char>) -> ShownAs {
        let shown = if is_default_ignorable(character) {
            self.marks = None;
            let joins = character == ZERO_WIDTH_JOINER
                && self.previous.is_some_and(is_pictographic)
                && next.is_some_and(is_pictographic);
            if joins {
                ShownAs::Written
            } else {
                ShownAs::Escaped
            }
        } else if is_mark(character) {
            self.marks = self.marks.map(|count| count + 1);
            if self.marks.is_none_or(|count| count > MARKS_PER_BASE) {
                ShownAs::Escaped
            } else {
                ShownAs::Usual
            }
        } else {
            self.marks = is_base(character).then_some(0);
            ShownAs::Usual
        };
        self.previous = Some(character);
        shown
    }
}

/// Whether `character` is Default_Ignorable_Code_Point.
fn is_default_ignorable(character: char) -> bool {
    DEFAULT_IGNORABLE
        .iter()
        .any(|ignorable| ignorable.contains(&character))
}

/// Whether `character` is a pictograph an emoji sequence joins: an emoji
/// that is no emoji component, which leaves out the digits, `#`, `*`,
/// skin tones and regional indicators. It stands for Extended_Pictographic,
/// which `unicode-properties` does not carry, short of its reserved code
/// points.
fn is_pictographic(character: char) -> bool {
    character.is_emoji_char() && !character.is_emoji_component()
}

/// Whether `character` is a combining mark, drawn on the character before it.
fn is_mark(character: char) -> bool {
    matches!(
        character.general_category(),
        GeneralCategory::NonspacingMark | GeneralCategory::EnclosingMark
    )
}

/// Whether `character` has no meaning or glyph anyone agreed on: unassigned,
/// or for private use, so it shows as nothing or as anything.
pub fn is_unknown(character: char) -> bool {
    matches!(
        character.general_category(),
        GeneralCategory::Unassigned | GeneralCategory::PrivateUse
    )
}

/// Whether `character` shows as itself and can carry the marks after it:
/// not blank, not unknown, and escaped by no rendering.
fn is_base(character: char) -> bool {
    !character.is_whitespace()
        && !is_unknown(character)
        && !steers_the_terminal(character)
        && !ANGLES.contains(&character)
}
