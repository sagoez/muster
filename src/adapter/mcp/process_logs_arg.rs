use getset::{CopyGetters, Getters};
use schemars::JsonSchema;
use serde::Deserialize;
use typed_builder::TypedBuilder;

/// Which part of a process's log `process_logs` returns.
#[derive(Debug, Deserialize, JsonSchema, Getters, CopyGetters, TypedBuilder)]
#[serde(deny_unknown_fields)]
pub struct ProcessLogsArg {
    /// The process's name, as stack_status shows it.
    #[getset(get = "pub")]
    name: String,
    /// A registered project's name or a folder; defaults to the project
    /// containing the server's working directory.
    #[builder(default)]
    #[getset(get = "pub")]
    project: Option<String>,
    /// Most lines returned, default 100, at most 2000.
    #[builder(default)]
    #[getset(get_copy = "pub")]
    lines: Option<usize>,
    /// A cursor from an earlier result; only output after it is returned.
    #[builder(default)]
    #[getset(get = "pub")]
    since: Option<String>,
    /// Keep only lines containing this text; a regex with regex=true.
    #[builder(default)]
    #[getset(get = "pub")]
    grep: Option<String>,
    /// Read grep as a regex.
    #[builder(default)]
    #[getset(get_copy = "pub")]
    regex: Option<bool>,
    /// Keep only error lines, each with its stack trace.
    #[builder(default)]
    #[getset(get_copy = "pub")]
    errors_only: Option<bool>,
    /// Most bytes returned, default 16384, at most 65536.
    #[builder(default)]
    #[getset(get_copy = "pub")]
    max_bytes: Option<usize>,
}
