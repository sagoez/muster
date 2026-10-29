use getset::{CopyGetters, Getters};
use schemars::JsonSchema;
use serde::Deserialize;
use typed_builder::TypedBuilder;

/// The todo `todo_claim` takes, and for how long.
#[derive(Debug, Default, Deserialize, JsonSchema, Getters, CopyGetters, TypedBuilder)]
#[serde(deny_unknown_fields)]
pub struct TodoClaimArg {
    /// A registered project's name or a folder; defaults to the project
    /// containing the server's working directory.
    #[builder(default)]
    #[getset(get = "pub")]
    project: Option<String>,
    /// The todo's id; without it, the oldest claimable todo.
    #[builder(default)]
    #[getset(get = "pub")]
    id: Option<String>,
    /// Seconds the claim lasts, default 1800, at most 28800.
    #[builder(default)]
    #[getset(get_copy = "pub")]
    lease_s: Option<u64>,
}
