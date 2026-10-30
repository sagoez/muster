use serde::Serialize;
use strum::Display;

/// What adding or removing muster's entry did to one config.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Display, Serialize)]
#[strum(serialize_all = "kebab-case")]
#[serde(rename_all = "kebab-case")]
pub enum EditOutcome {
    /// The entry was added.
    Added,
    /// The same entry was already there, so nothing was written.
    AlreadyAdded,
    /// A different entry under the name was replaced, as `--force` asked.
    Replaced,
    /// The entry was removed.
    Removed,
    /// There was no entry under the name, so nothing was written.
    Absent,
}

impl EditOutcome {
    /// Whether the config file is written.
    pub fn writes(self) -> bool {
        matches!(self, Self::Added | Self::Replaced | Self::Removed)
    }
}
