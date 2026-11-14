/// What became of one byte fed to a
/// [`BoundedParser`](super::bounded_parser::BoundedParser).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fed {
    /// The parser took it.
    Kept,
    /// It belongs to a sequence that ran past
    /// [`MAX_PENDING_BYTES`](super::bounded_parser::MAX_PENDING_BYTES),
    /// which is dropped up to its terminator.
    Dropped,
}
