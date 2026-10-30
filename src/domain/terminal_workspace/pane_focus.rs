use strum::{Display, EnumIter, EnumString};

/// Whether a new pane takes the user's focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Display, EnumString, EnumIter)]
#[strum(serialize_all = "kebab-case")]
pub enum PaneFocus {
    /// The new pane is focused, as when the user asked to see it.
    Take,
    /// The focus stays where it is, as when an agent opens it.
    Keep,
}
