use getset::{CopyGetters, Getters};
use schemars::JsonSchema;
use serde::Deserialize;
use typed_builder::TypedBuilder;

/// The lock `lock_acquire` takes.
#[derive(Debug, Deserialize, JsonSchema, Getters, CopyGetters, TypedBuilder)]
#[serde(deny_unknown_fields)]
pub struct LockAcquireArg {
    /// A registered project's name or a folder; defaults to the project
    /// containing the server's working directory.
    #[builder(default)]
    #[getset(get = "pub")]
    project: Option<String>,
    /// The lock's name: 1 to 64 of a-z, 0-9, '.', '_' and '-'.
    #[getset(get = "pub")]
    name: String,
    /// Seconds until it lapses, default 300, at most 3600.
    #[builder(default)]
    #[getset(get_copy = "pub")]
    ttl_s: Option<u64>,
    /// Why you hold it, shown to whoever finds it held.
    #[builder(default)]
    #[getset(get = "pub")]
    note: Option<String>,
    /// Seconds to wait for it to free up, at most 45.
    #[builder(default)]
    #[getset(get_copy = "pub")]
    wait_s: Option<u64>,
}
