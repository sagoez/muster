use std::{io, path::PathBuf};

use getset::Getters;
use typed_builder::TypedBuilder;

/// One exchange with the service that failed, with the socket it was
/// resolved to, so the failure names that socket without resolving it again.
#[derive(Debug, Getters, TypedBuilder)]
#[getset(get = "pub(super)")]
pub(super) struct ExchangeFailure {
    /// The socket the exchange went to, or would have.
    socket: PathBuf,
    /// Why it failed.
    source: io::Error,
}

impl ExchangeFailure {
    /// The failure, taken apart into its socket and why.
    pub(super) fn into_parts(self) -> (PathBuf, io::Error) {
        (self.socket, self.source)
    }
}
