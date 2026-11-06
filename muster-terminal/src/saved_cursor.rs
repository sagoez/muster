use getset::CopyGetters;
use typed_builder::TypedBuilder;

use super::pen::Pen;

/// The cursor as a save leaves it, for a later restore.
#[derive(Clone, Copy, Debug, PartialEq, Eq, CopyGetters, TypedBuilder)]
#[getset(get_copy = "pub")]
pub struct SavedCursor {
    /// The column.
    column: usize,
    /// The pen.
    pen: Pen,
    /// How far above the newest line the cursor stood.
    risen: usize,
}
