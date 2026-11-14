use nutype::nutype;

/// Which entry of the edit log an edit is. A later edit has a larger id, so
/// a reader that has seen up to one id knows every edit after it is new.
#[nutype(derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Display))]
pub struct EditId(i64);
