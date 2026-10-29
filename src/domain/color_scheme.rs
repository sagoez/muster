use serde::{Deserialize, Serialize};
use strum::Display;

/// The colors a managed terminal reports when a child asks for its default
/// foreground and background, so an agent themes for the user's terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Display)]
#[serde(rename_all = "kebab-case")]
#[strum(serialize_all = "kebab-case")]
pub enum ColorScheme {
    /// A dark background with light text.
    Dark,
    /// A light background with dark text.
    Light,
}
