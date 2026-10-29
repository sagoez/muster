use super::cell_style::CellStyle;

/// One grid cell as a test compares it: its glyph, combining characters,
/// resolved style, and whether it is the trailing half of a wide character.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderCell {
    pub(super) ch: char,
    pub(super) zerowidth: Vec<char>,
    pub(super) style: CellStyle,
    pub(super) spacer: bool,
}

impl RenderCell {
    /// The glyph in the cell.
    pub fn ch(&self) -> char {
        self.ch
    }

    /// Combining characters attached to the glyph.
    pub fn zerowidth(&self) -> &[char] {
        &self.zerowidth
    }
}
