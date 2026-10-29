use getset::{CopyGetters, Getters};
use serde::{Deserialize, Serialize};
use typed_builder::TypedBuilder;

use super::url_source::UrlSource;

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
