/// Identity of one connection, unique for the holder's lifetime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct ClientId(u64);

impl ClientId {
    /// The connection numbered `number`.
    pub(super) fn new(number: u64) -> Self {
        Self(number)
    }
}
