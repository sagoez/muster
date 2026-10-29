use getset::Getters;
use schemars::JsonSchema;
use serde::Deserialize;
use typed_builder::TypedBuilder;

/// A new todo's title and its dependency ids.
#[derive(Debug, Deserialize, JsonSchema, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct TodoAddArg {
    /// What the task is.
    title: String,
    /// Ids of todos this one depends on.
    #[serde(default)]
    #[builder(default)]
    deps: Vec<String>,
}
