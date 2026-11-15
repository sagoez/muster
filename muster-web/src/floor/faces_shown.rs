use super::view::Unconfirmed;

/// How a floor draws its processes: held back while no status reading has
/// placed them, as last known while nothing confirms them, or confirmed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FacesShown {
    /// None is drawn, since none has a room yet.
    Held,
    /// Each is drawn as last read, unconfirmed for the reason it holds.
    Unconfirmed(Unconfirmed),
    /// Each is drawn as the service last answered of it.
    Confirmed,
}

impl FacesShown {
    /// Why the faces drawn this way are unconfirmed, none when they are
    /// confirmed; a held face is one no status has been read for yet.
    pub(super) fn unconfirmed(self) -> Option<Unconfirmed> {
        match self {
            Self::Held => Some(Unconfirmed::Resting),
            Self::Unconfirmed(why) => Some(why),
            Self::Confirmed => None,
        }
    }
}
