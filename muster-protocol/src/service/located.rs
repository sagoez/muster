use std::path::PathBuf;

use getset::Getters;
use serde::{Deserialize, Serialize};
use typed_builder::TypedBuilder;

/// Where a running process can be reached directly: the socket of the holder
/// that runs it, which `attach` connects to, and its output log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Getters, TypedBuilder)]
#[getset(get = "pub")]
#[serde(rename_all = "kebab-case")]
pub struct Located {
    /// The holder's socket.
    socket: PathBuf,
    /// The process's output log.
    log: PathBuf,
}
