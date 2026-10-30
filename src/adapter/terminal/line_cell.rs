use getset::{CopyGetters, Getters};
use typed_builder::TypedBuilder;

use super::pen::Pen;

/// One column of a rendered log line: a character and the pen it was drawn
/// in.
#[derive(Clone, Debug, PartialEq, Eq, Getters, CopyGetters, TypedBuilder)]
pub struct LineCell {
    /// The character.
    #[getset(get_copy = "pub")]
    character: char,
    /// Its colour and attributes.
    #[getset(get = "pub")]
    pen: Pen,
}
