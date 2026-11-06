use std::io;

use getset::Getters;
use typed_builder::TypedBuilder;

/// Whether the directories this user's service is reached through are
/// private to them, as `muster doctor` reports it.
#[derive(Debug, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct SocketDirCheck {
    /// Why the directory of the socket a service published is not private,
    /// so it is passed over; none when it is, or nothing is published.
    published: Option<io::Error>,
    /// Why the directory of this client's own socket is not private, so no
    /// request is sent there; none when it is, or does not exist yet.
    own: Option<io::Error>,
}
