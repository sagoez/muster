use serde::Serialize;
use strum::Display;

/// What is done to muster's entry in an agent's config.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Display, Serialize)]
#[strum(serialize_all = "kebab-case")]
#[serde(rename_all = "kebab-case")]
pub enum EntryChange {
    /// The entry is added.
    Add,
    /// The entry is removed.
    Remove,
}
