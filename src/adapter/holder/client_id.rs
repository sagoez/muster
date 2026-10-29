/// Identity of one connection, unique for the holder's lifetime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct ClientId(pub(super) u64);
