use getset::{CopyGetters, Getters};
use schemars::JsonSchema;
use serde::Deserialize;
use typed_builder::TypedBuilder;

use super::wait_arg::WaitArg;

/// A start or restart, and whether it waits.
#[derive(Debug, Deserialize, JsonSchema, Getters, CopyGetters, TypedBuilder)]
#[serde(deny_unknown_fields)]
pub struct ControlArg {
    /// The process's name, as stack_status shows it.
    #[getset(get = "pub")]
    name: String,
    /// A registered project's name or a folder; defaults to the project
    /// containing the server's working directory.
    #[builder(default)]
    #[getset(get = "pub")]
    project: Option<String>,
    /// true waits until it is ready (or running without a readiness check);
    /// a condition as process_wait takes waits for that.
    #[builder(default)]
    #[getset(get = "pub")]
    wait: Option<WaitArg>,
    /// Seconds to wait, default 30, at most 45.
    #[builder(default)]
    #[getset(get_copy = "pub")]
    timeout_s: Option<u64>,
}
