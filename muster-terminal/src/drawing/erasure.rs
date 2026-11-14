use getset::CopyGetters;
use typed_builder::TypedBuilder;

/// One erase to the cursor not yet applied to the cells it covers: every
/// cell left of `extent` drawn before `stamp` reads as blank.
#[derive(Clone, Copy, Debug, PartialEq, Eq, CopyGetters, TypedBuilder)]
#[getset(get_copy = "pub")]
pub struct Erasure {
    /// The stamp of the erase; a cell drawn at it or later is newer.
    stamp: u32,
    /// The first column the erase leaves alone.
    extent: usize,
}
