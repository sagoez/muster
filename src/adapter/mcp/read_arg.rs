use getset::{CopyGetters, Getters};
use schemars::JsonSchema;
use serde::Deserialize;
use typed_builder::TypedBuilder;

/// One note or key-value entry by key, or a page of them without one.
#[derive(Debug, Default, Deserialize, JsonSchema, Getters, CopyGetters, TypedBuilder)]
#[serde(deny_unknown_fields)]
pub struct ReadArg {
    /// A registered project's name or a folder; defaults to the project
    /// containing the server's working directory.
    #[builder(default)]
    #[getset(get = "pub")]
    project: Option<String>,
    /// The entry's key; without it, the entries are listed.
    #[builder(default)]
    #[getset(get = "pub")]
    key: Option<String>,
    /// Most entries listed, default <default-list-limit>, at most <max-list-limit>.
    #[builder(default)]
    #[getset(get_copy = "pub")]
    limit: Option<usize>,
}
