use getset::{CopyGetters, Getters};
use typed_builder::TypedBuilder;

use super::cell_style::CellStyle;

/// One captured cell.
#[derive(Debug, Clone, Getters, CopyGetters, TypedBuilder)]
pub struct SnapshotCell {
    /// The base glyph.
    #[getset(get_copy = "pub")]
    ch: char,
    /// Combining characters attached to the glyph.
    #[getset(get = "pub")]
    zerowidth: Vec<char>,
    /// Resolved colors and attributes.
    #[getset(get_copy = "pub")]
    style: CellStyle,
    /// Whether this is the trailing half of a wide character.
    #[getset(get_copy = "pub")]
    spacer: bool,
}

impl SnapshotCell {
    /// Whether the cell is an unstyled blank a reset screen already shows.
    pub(super) fn is_blank_default(&self) -> bool {
        !self.spacer
            && self.ch == ' '
            && self.zerowidth.is_empty()
            && self.style == CellStyle::default()
    }
}
