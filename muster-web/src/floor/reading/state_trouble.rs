use super::health::Health;

/// What kept muster's state from being read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StateTrouble {
    /// muster has no database yet.
    NoState,
    /// An older muster wrote the database, which the next muster command
    /// migrates.
    OlderSchema,
    /// A newer muster wrote the database.
    NewerSchema,
    /// The database could not be read this time.
    Unreadable,
}

impl From<StateTrouble> for Health {
    /// The health the page shows for a state in `trouble`.
    fn from(trouble: StateTrouble) -> Self {
        match trouble {
            StateTrouble::NoState => Self::NoState,
            StateTrouble::OlderSchema => Self::OlderSchema,
            StateTrouble::NewerSchema => Self::NewerSchema,
            StateTrouble::Unreadable => Self::Unreadable,
        }
    }
}
