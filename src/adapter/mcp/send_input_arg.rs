use getset::{CopyGetters, Getters};
use schemars::JsonSchema;
use serde::Deserialize;
use typed_builder::TypedBuilder;

/// Text to type into a process's terminal.
#[derive(Debug, Deserialize, JsonSchema, Getters, CopyGetters, TypedBuilder)]
pub struct SendInputArg {
    /// The process's name, as `process_list` shows it.
    #[getset(get = "pub")]
    name: String,
    /// Text to write to the process's terminal.
    #[getset(get = "pub")]
    text: String,
    /// Append a carriage return to submit the line (run a command, answer a
    /// prompt). Defaults to false, so the text is typed without submitting.
    #[serde(default)]
    #[builder(default)]
    #[getset(get_copy = "pub")]
    enter: bool,
}
