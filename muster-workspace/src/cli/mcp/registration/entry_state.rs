use serde_json::Value;

/// What one scope's servers file says of muster's entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EntryState {
    /// The scope holds muster, as this entry.
    Held(Value),
    /// The scope holds no muster entry.
    Absent,
    /// The scope's servers file could not be read, so muster cannot tell.
    Unknown,
}
