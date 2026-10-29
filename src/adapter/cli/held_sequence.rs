/// What the bytes a [`DetachScanner`](super::DetachScanner) held since an
/// escape have turned out to be.
pub(super) enum HeldSequence {
    /// Still possibly the detach key.
    Partial,
    /// The detach key.
    Detach,
    /// A whole sequence that is something else.
    Other,
    /// Not a sequence the detach key could be; its last byte starts afresh.
    Interrupted,
}
