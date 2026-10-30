use getset::Getters;
use schemars::JsonSchema;
use serde::Deserialize;
use typed_builder::TypedBuilder;

/// A todo by its id.
#[derive(Debug, Deserialize, JsonSchema, Getters, TypedBuilder)]
#[serde(deny_unknown_fields)]
#[getset(get = "pub")]
pub struct TodoIdArg {
    /// A registered project's name or a folder; defaults to the project
    /// containing the server's working directory.
    #[builder(default)]
    project: Option<String>,
    /// The todo's id.
    id: String,
}
