use serde::Serialize;

/// What adding or removing muster's entry did to one config.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum EditOutcome {
    /// The entry was added.
    Added,
    /// The same entry was already there, so nothing was written.
    AlreadyAdded,
    /// muster's entry from another `muster` path was pointed at this one.
    Updated,
    /// muster's entry is there but turned off, so nothing was written;
    /// `--force` turns it back on.
    Disabled,
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
        matches!(
            self,
            Self::Added | Self::Updated | Self::Replaced | Self::Removed
        )
    }
}
