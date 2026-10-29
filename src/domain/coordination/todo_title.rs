use nutype::nutype;

/// A todo's human-readable title. Trimmed and non-empty, so a blank line can
/// never masquerade as a task.
#[nutype(
    sanitize(trim),
    validate(not_empty),
    derive(
        Debug,
        Clone,
        PartialEq,
        Eq,
        Hash,
        AsRef,
        Display,
        Serialize,
        Deserialize
    )
)]
pub struct TodoTitle(String);
