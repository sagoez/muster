use nutype::nutype;

/// How many bytes a whole note body takes, as stored.
#[nutype(derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Display))]
pub struct NoteBodyBytes(usize);
