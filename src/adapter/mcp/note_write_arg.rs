use getset::Getters;
use schemars::JsonSchema;
use serde::Deserialize;
use typed_builder::TypedBuilder;

/// A note's key and its new body.
#[derive(Debug, Deserialize, JsonSchema, Getters, TypedBuilder)]
#[serde(deny_unknown_fields)]
#[getset(get = "pub")]
pub struct NoteWriteArg {
    /// A registered project's name or a folder; defaults to the project
    /// containing the server's working directory.
    #[builder(default)]
    project: Option<String>,
    /// The note's key.
    key: String,
    /// Its markdown body.
    body: String,
}
