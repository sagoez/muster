use nutype::nutype;

/// A count of terminal columns.
#[nutype(derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Display))]
pub struct Cols(u16);
