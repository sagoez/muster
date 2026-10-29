use crate::{
    adapter::probe::{probe_http, probe_port},
    domain::value::{LocalPort, LocalUrl},
};

/// What a probe tries.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ProbeTarget {
    /// A TCP connect to the local port.
    Port(LocalPort),
    /// An HTTP GET of the local URL, passing on `status` or below 500.
    Http {
        /// What to fetch.
        url: LocalUrl,
        /// The status to pass on.
        status: Option<u16>,
    },
}

impl ProbeTarget {
    /// Tries it once.
    pub(super) fn passes(&self) -> bool {
        match self {
            Self::Port(port) => probe_port(*port),
            Self::Http { url, status } => probe_http(url, *status),
        }
    }
}
