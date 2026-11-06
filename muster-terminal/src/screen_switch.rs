use getset::CopyGetters;
use typed_builder::TypedBuilder;

/// A point in one chunk of a child's output where the screen it draws on
/// changed: the alternate screen came up or went away.
#[derive(Debug, Clone, Copy, PartialEq, Eq, CopyGetters, TypedBuilder)]
#[getset(get_copy = "pub")]
pub struct ScreenSwitch {
    /// The index in the chunk just past the byte that made the switch.
    at: usize,
    /// Whether the alternate screen is up from there on.
    alternate: bool,
}
