use getset::Getters;
use schemars::JsonSchema;
use serde::Deserialize;
use typed_builder::TypedBuilder;

/// A call that takes only the project, such as `stack_status`.
#[derive(Debug, Default, Deserialize, JsonSchema, Getters, TypedBuilder)]
#[serde(deny_unknown_fields)]
#[getset(get = "pub")]
pub struct ProjectArg {
    /// A registered project's name or a folder; defaults to the project
    /// containing the server's working directory.
    #[builder(default)]
    project: Option<String>,
}
