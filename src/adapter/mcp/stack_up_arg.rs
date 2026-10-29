use getset::{CopyGetters, Getters};
use schemars::JsonSchema;
use serde::Deserialize;
use typed_builder::TypedBuilder;

/// What `stack_up` starts and whether it waits.
#[derive(Debug, Default, Deserialize, JsonSchema, Getters, CopyGetters, TypedBuilder)]
#[serde(deny_unknown_fields)]
pub struct StackUpArg {
    /// A registered project's name or a folder; defaults to the project
    /// containing the server's working directory.
    #[builder(default)]
    #[getset(get = "pub")]
    project: Option<String>,
    /// Wait until each started process is ready, or running when it has no
    /// readiness check.
    #[builder(default)]
    #[getset(get_copy = "pub")]
    wait: Option<bool>,
    /// Seconds to wait, default 30, at most 45.
    #[builder(default)]
    #[getset(get_copy = "pub")]
    timeout_s: Option<u64>,
}
