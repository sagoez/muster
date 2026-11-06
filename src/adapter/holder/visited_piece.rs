use getset::{CopyGetters, Getters};
use typed_builder::TypedBuilder;

use crate::domain::value::OutputCursor;

/// One raw line of a log, or the piece of one a read cut it into, as a scan
/// of the log hands it out with the lines its rendering finished.
#[derive(Clone, Debug, PartialEq, Eq, Getters, CopyGetters, TypedBuilder)]
pub(super) struct VisitedPiece<'a> {
    /// Where the raw piece begins in the output.
    #[getset(get_copy = "pub(super)")]
    at: OutputCursor,
    /// The raw bytes of the piece.
    #[getset(get_copy = "pub(super)")]
    bytes: &'a [u8],
    /// Where the first of `texts` began in the output: an earlier piece when
    /// that line was drawn across several.
    #[getset(get_copy = "pub(super)")]
    line_at: OutputCursor,
    /// The text of each line the piece finished, oldest first.
    #[getset(get_copy = "pub(super)")]
    texts: &'a [String],
}

impl VisitedPiece<'_> {
    /// Where the line at `index` of the texts began: the first where its
    /// drawing began, a later one within this piece.
    pub(super) fn began(&self, index: usize) -> OutputCursor {
        if index == 0 { self.line_at } else { self.at }
    }
}
