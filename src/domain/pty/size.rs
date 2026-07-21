use getset::CopyGetters;
use typed_builder::TypedBuilder;

use crate::domain::value::{Cols, Rows};

/// Dimensions of a pseudo-terminal, in character cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq, CopyGetters, TypedBuilder)]
#[getset(get_copy = "pub")]
pub struct PtySize {
    rows: Rows,
    cols: Cols,
}
