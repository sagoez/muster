use nutype::nutype;

/// A count of terminal rows.
#[nutype(derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Display))]
pub struct Rows(u16);
