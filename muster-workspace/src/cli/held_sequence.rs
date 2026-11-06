/// What the bytes a [`DetachScanner`](super::DetachScanner) held since an
/// escape have turned out to be.
pub(super) enum HeldSequence {
    /// Still possibly the detach key.
    Partial,
    /// The detach key.
    Detach,
    /// Something else, passed on as it is.
    Other,
}
