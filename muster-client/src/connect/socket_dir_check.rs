use std::io;

use getset::Getters;
use muster_protocol::holder::HolderError;
use typed_builder::TypedBuilder;

/// Whether the directories this user's service and its holders are reached
/// through are private to them, as `muster doctor` reports it.
#[derive(Debug, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct SocketDirCheck {
    /// Why the directory of the socket a service published is not private,
    /// so it is passed over; none when it is, or nothing is published.
    published: Option<io::Error>,
    /// Why the directory of this client's own socket is not private, so no
    /// request is sent there; none when it is, or does not exist yet.
    own: Option<io::Error>,
    /// Why the folder in `/tmp/muster-<uid>` holder sockets go in, or that
    /// directory itself, is not private, so no holder can be launched; none
    /// when it is, or does not exist yet.
    holders: Option<HolderError>,
}
