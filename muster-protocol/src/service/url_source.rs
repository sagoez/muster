use serde::{Deserialize, Serialize};
use strum::Display;

/// Where a process's URL comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Display, Serialize, Deserialize)]
#[strum(serialize_all = "kebab-case")]
#[serde(rename_all = "kebab-case")]
pub enum UrlSource {
    /// The process's readiness check names it.
    Declared,
    /// A port the process was found listening on.
    Detected,
}
