use getset::{CopyGetters, Getters};
use schemars::JsonSchema;
use serde::Deserialize;
use typed_builder::TypedBuilder;

use super::condition_arg::ConditionArg;

/// What `process_wait` waits for.
#[derive(Debug, Deserialize, JsonSchema, Getters, CopyGetters, TypedBuilder)]
#[serde(deny_unknown_fields)]
pub struct ProcessWaitArg {
    /// The process's name, as stack_status shows it.
    #[getset(get = "pub")]
    name: String,
    /// A registered project's name or a folder; defaults to the project
    /// containing the server's working directory.
    #[builder(default)]
    #[getset(get = "pub")]
    project: Option<String>,
    /// The condition; defaults to ready when the process has a readiness
    /// check, else running.
    #[builder(default)]
    #[getset(get = "pub")]
    until: Option<ConditionArg>,
    /// A cursor from an earlier result; only output after it counts.
    #[builder(default)]
    #[getset(get = "pub")]
    since: Option<String>,
    /// Seconds to wait, default <default-wait-s>, at most <call-bound-s>.
    #[builder(default)]
    #[getset(get_copy = "pub")]
    timeout_s: Option<u64>,
}
