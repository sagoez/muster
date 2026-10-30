use getset::CopyGetters;
use typed_builder::TypedBuilder;

use super::{cell_color::CellColor, cell_flags::CellFlags};

/// The resolved colors and attributes of one cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, CopyGetters, TypedBuilder)]
pub struct CellStyle {
    /// Foreground color.
    #[getset(get_copy = "pub")]
    fg: CellColor,
    /// Background color.
    #[getset(get_copy = "pub")]
    bg: CellColor,
    /// Text attributes.
    #[getset(get_copy = "pub")]
    flags: CellFlags,
}

impl Default for CellStyle {
    /// The unstyled cell: default colors and no attributes.
    fn default() -> Self {
        Self::builder()
            .fg(CellColor::Default)
            .bg(CellColor::Default)
            .flags(CellFlags::default())
            .build()
    }
}
