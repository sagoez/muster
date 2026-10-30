use getset::{CopyGetters, Getters};
use schemars::JsonSchema;
use serde::Deserialize;
use typed_builder::TypedBuilder;

/// Text to type into a process's terminal.
#[derive(Debug, Deserialize, JsonSchema, Getters, CopyGetters, TypedBuilder)]
#[serde(deny_unknown_fields)]
pub struct SendInputArg {
    /// The process's name, as stack_status shows it.
    #[getset(get = "pub")]
    name: String,
    /// A registered project's name or a folder; defaults to the project
    /// containing the server's working directory.
    #[builder(default)]
    #[getset(get = "pub")]
    project: Option<String>,
    /// Text to write to the process's terminal.
    #[getset(get = "pub")]
    text: String,
    /// Append a carriage return to submit the line. Defaults to false, so
    /// the text is typed without submitting.
    #[builder(default)]
    #[getset(get_copy = "pub")]
    enter: Option<bool>,
}
