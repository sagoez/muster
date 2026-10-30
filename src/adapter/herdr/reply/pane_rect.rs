use getset::CopyGetters;
use serde::Deserialize;
use typed_builder::TypedBuilder;

/// A pane's size, in cells.
#[derive(Debug, Deserialize, CopyGetters, TypedBuilder)]
#[getset(get_copy = "pub")]
pub struct PaneRect {
    /// Columns.
    width: u16,
    /// Rows.
    height: u16,
}
