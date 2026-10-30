use getset::CopyGetters;
use serde::Deserialize;

/// A pane's size, in cells.
#[derive(Debug, Deserialize, CopyGetters)]
#[getset(get_copy = "pub")]
pub struct PaneRect {
    /// Columns.
    width: u16,
    /// Rows.
    height: u16,
}
