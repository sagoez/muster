use getset::{CopyGetters, Getters};
use schemars::JsonSchema;
use serde::Deserialize;
use typed_builder::TypedBuilder;

/// Which events `stack_events` returns and how long it waits for one.
#[derive(Debug, Default, Deserialize, JsonSchema, Getters, CopyGetters, TypedBuilder)]
#[serde(deny_unknown_fields)]
pub struct StackEventsArg {
    /// A registered project's name or a folder; defaults to the project
    /// containing the server's working directory.
    #[builder(default)]
    #[getset(get = "pub")]
    project: Option<String>,
    /// The cursor from an earlier result; only later events are returned.
    #[builder(default)]
    #[getset(get = "pub")]
    since: Option<String>,
    /// Only these kinds: started, ready, ready-timeout, exited, crashed,
    /// restarting, stopped, blocked, notice.
    #[builder(default)]
    #[getset(get = "pub")]
    kinds: Option<Vec<String>>,
    /// Seconds to wait for a new event, at most 45; without it the call
    /// answers at once.
    #[builder(default)]
    #[getset(get_copy = "pub")]
    wait_s: Option<u64>,
}
