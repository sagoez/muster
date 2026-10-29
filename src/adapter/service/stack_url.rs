use getset::{CopyGetters, Getters};
use serde::{Deserialize, Serialize};
use strum::Display;
use typed_builder::TypedBuilder;

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

/// A URL a process serves.
#[derive(
    Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Getters, CopyGetters, TypedBuilder,
)]
#[serde(rename_all = "kebab-case")]
pub struct StackUrl {
    /// The URL.
    #[getset(get = "pub")]
    url: String,
    /// Where it comes from.
    #[getset(get_copy = "pub")]
    source: UrlSource,
}
