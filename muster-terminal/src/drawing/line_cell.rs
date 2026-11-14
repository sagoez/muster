use getset::CopyGetters;
use typed_builder::TypedBuilder;

use super::pen::Pen;

/// What an erased column holds until something overwrites it.
pub const BLANK: char = ' ';

/// One column of a log line: the glyph that starts there, or the rest of a
/// glyph wider than one column, with the pen it was drawn in. A `Copy`
/// value, so filling, erasing and dropping cells allocates and counts
/// nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, CopyGetters, TypedBuilder)]
#[getset(get_copy = "pub")]
pub struct LineCell {
    /// The character, or [`BLANK`] in a glyph's later columns.
    character: char,
    /// Its color and attributes.
    pen: Pen,
    /// Columns the glyph starting here spans; zero for a later column of a
    /// wider glyph.
    width: u8,
    /// When it was drawn, against which an erase to the cursor is checked.
    stamp: u32,
}

impl LineCell {
    /// An erased column, drawn at `stamp`.
    pub fn blank(stamp: u32) -> Self {
        Self {
            character: BLANK,
            pen: Pen::default(),
            width: 1,
            stamp,
        }
    }

    /// A later column of a glyph drawn in `pen` at `stamp`.
    pub fn continuation(pen: Pen, stamp: u32) -> Self {
        Self {
            character: BLANK,
            pen,
            width: 0,
            stamp,
        }
    }

    /// The same cell drawn at `stamp`.
    pub fn restamped(self, stamp: u32) -> Self {
        Self { stamp, ..self }
    }

    /// Whether this is a later column of a wider glyph.
    pub fn continues(&self) -> bool {
        self.width == 0
    }
}
