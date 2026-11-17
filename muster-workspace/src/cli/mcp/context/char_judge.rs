use std::ops::RangeInclusive;

use muster_domain::value::steers_the_terminal;
use unicode_properties::{GeneralCategory, UnicodeEmoji, UnicodeGeneralCategory};

use crate::cli::mcp::context::shown_as::ShownAs;

/// The most combining marks one base character keeps as themselves: enough
/// for fully pointed Hebrew and any script's stacked vowels and tones, while
/// a longer run, which can smear over other text, shows the rest as escapes.
pub const MARKS_PER_BASE: usize = 6;
/// The most variation selectors one piece of shared text keeps as
/// themselves, at most one per base character: each can carry a hidden
/// byte, so past these they show as escapes.
pub const SELECTORS_PER_TEXT: usize = 8;
/// The variation selectors, which pick a base character's glyph and draw
/// nothing of their own.
const VARIATION_SELECTORS: [RangeInclusive<char>; 2] =
    ['\u{fe00}'..='\u{fe0f}', '\u{e0100}'..='\u{e01ef}'];
/// The zero-width joiner, which joins emoji into one glyph.
const ZERO_WIDTH_JOINER: char = '\u{200d}';
/// The zero-width non-joiner, which keeps two letters apart, as Persian
/// writes them.
const ZERO_WIDTH_NON_JOINER: char = '\u{200c}';
/// The angle brackets, which shared text always shows as escapes.
pub const ANGLES: [char; 2] = ['<', '>'];
/// Letters that show as blank space.
pub const BLANK_LETTERS: [char; 5] = ['\u{115f}', '\u{1160}', '\u{3164}', '\u{ffa0}', '\u{2800}'];

/// Judges each character of one piece of shared text by what came before
/// it and what comes next, so marks and selectors show only on a base and
/// joiners only between what they join. A private helper of
/// [`Rendering::fitted`](super::rendering::Rendering::fitted), built with
/// [`Default`].
#[derive(Debug, Default)]
pub struct CharJudge {
    /// The marks on the current base, none when nothing before can carry one.
    marks: Option<usize>,
    /// Whether the current base already has its variation selector.
    selected: bool,
    /// The variation selectors kept so far.
    selectors: usize,
    /// The character before, if any.
    previous: Option<char>,
}

impl CharJudge {
    /// How `character`, followed by `next`, shows.
    pub fn judge(&mut self, character: char, next: Option<char>) -> ShownAs {
        let shown = if is_selector(character) {
            if self.marks.is_none() || self.selected || self.selectors >= SELECTORS_PER_TEXT {
                ShownAs::Escaped
            } else {
                self.selected = true;
                self.selectors += 1;
                ShownAs::Usual
            }
        } else if is_mark(character) {
            self.marks = self.marks.map(|count| count + 1);
            if self.marks.is_none_or(|count| count > MARKS_PER_BASE) {
                ShownAs::Escaped
            } else {
                ShownAs::Usual
            }
        } else if self.joins(character, next) {
            self.marks = None;
            ShownAs::Written
        } else {
            self.marks = is_base(character).then_some(0);
            self.selected = false;
            ShownAs::Usual
        };
        self.previous = Some(character);
        shown
    }

    /// Whether `character` is a joiner between two characters it joins: a
    /// zero-width joiner between emoji, or a non-joiner between letters.
    fn joins(&self, character: char, next: Option<char>) -> bool {
        let (Some(previous), Some(next)) = (self.previous, next) else {
            return false;
        };
        match character {
            ZERO_WIDTH_JOINER => {
                previous.is_emoji_char_or_emoji_component() && next.is_emoji_char()
            },
            ZERO_WIDTH_NON_JOINER => {
                (previous.is_alphabetic() || is_mark(previous)) && next.is_alphabetic()
            },
            _ => false,
        }
    }
}

/// Whether `character` is a variation selector.
fn is_selector(character: char) -> bool {
    VARIATION_SELECTORS
        .iter()
        .any(|selectors| selectors.contains(&character))
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
        && !BLANK_LETTERS.contains(&character)
        && !is_unknown(character)
        && !steers_the_terminal(character)
        && !ANGLES.contains(&character)
}
