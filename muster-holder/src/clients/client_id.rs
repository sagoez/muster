/// Identity of one connection, unique for the holder's lifetime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct ClientId(u64);

impl ClientId {
    /// The connection numbered `number`.
    pub(crate) fn new(number: u64) -> Self {
        Self(number)
    }
}
