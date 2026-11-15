/// How a floor draws its processes: held back while no status reading has
/// placed them, as last known while nothing confirms them, or confirmed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FacesShown {
    /// None is drawn, since none has a room yet.
    Held,
    /// Each is drawn as last read, unconfirmed.
    Unconfirmed,
    /// Each is drawn as the service last answered of it.
    Confirmed,
}
