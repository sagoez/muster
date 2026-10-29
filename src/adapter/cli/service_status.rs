use std::path::PathBuf;

use getset::{CopyGetters, Getters};
use serde::Serialize;
use typed_builder::TypedBuilder;

/// What `muster service status` reports, without starting the service.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Getters, CopyGetters, TypedBuilder)]
pub struct ServiceStatus {
    /// Whether a service answers.
    #[getset(get_copy = "pub")]
    running: bool,
    /// The running service's muster version.
    #[getset(get = "pub")]
    version: Option<String>,
    /// The muster binary the running service runs.
    #[getset(get = "pub")]
    binary: Option<PathBuf>,
    /// This binary's muster version, which replaces a service of another one
    /// at the next command.
    #[getset(get = "pub")]
    client_version: String,
    /// The socket the service listens on.
    #[getset(get = "pub")]
    socket: PathBuf,
    /// The log the service records its failures in.
    #[getset(get = "pub")]
    log: PathBuf,
    /// The config of every project the service has loaded.
    #[getset(get = "pub")]
    projects: Vec<PathBuf>,
}
