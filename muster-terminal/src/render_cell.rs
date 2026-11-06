use getset::{CopyGetters, Getters};
use typed_builder::TypedBuilder;

use super::cell_style::CellStyle;

/// One grid cell as a test compares it: its glyph, combining characters,
/// resolved style, and whether it is the trailing half of a wide character.
#[derive(Debug, Clone, PartialEq, Getters, CopyGetters, TypedBuilder)]
pub struct RenderCell {
    /// The glyph in the cell.
    #[getset(get_copy = "pub")]
    ch: char,
    /// Combining characters attached to the glyph.
    #[getset(get = "pub")]
    zerowidth: Vec<char>,
    /// The cell's resolved style.
    #[getset(get = "pub")]
    style: CellStyle,
    /// Whether the cell is the trailing half of a wide character.
    #[getset(get_copy = "pub")]
    spacer: bool,
}
