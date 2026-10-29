use serde::{Deserialize, Serialize};
use strum::{Display, EnumIter, EnumString};

/// The colors a managed terminal reports when a child asks for its default
/// foreground and background, so a child themes for the user's terminal.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Display, EnumString, EnumIter,
)]
#[serde(rename_all = "kebab-case")]
#[strum(serialize_all = "kebab-case")]
pub enum ColorScheme {
    /// A dark background with light text.
    Dark,
    /// A light background with dark text.
    Light,
}
