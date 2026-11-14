use nutype::nutype;

/// Longest a log pattern may be, in characters, so no agent can hand the
/// service a pattern that stalls it.
pub const PATTERN_MAX_CHARS: usize = 512;
/// What a pattern's text must be.
pub(super) const PATTERN_FORM: &str = "a pattern of 1 to 512 characters";

/// The text of a log pattern: trimmed, 1 to [`PATTERN_MAX_CHARS`] characters.
#[nutype(
    sanitize(trim),
    validate(not_empty, len_char_max = PATTERN_MAX_CHARS),
    derive(Debug, Clone, PartialEq, Eq, Hash, AsRef, Display, Serialize, Deserialize)
)]
pub struct PatternText(String);
