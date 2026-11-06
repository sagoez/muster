use getset::Getters;
use schemars::JsonSchema;
use serde::Deserialize;
use typed_builder::TypedBuilder;

/// A new todo's title and its dependency ids.
#[derive(Debug, Deserialize, JsonSchema, Getters, TypedBuilder)]
#[serde(deny_unknown_fields)]
#[getset(get = "pub")]
pub struct TodoAddArg {
    /// A registered project's name or a folder; defaults to the project
    /// containing the server's working directory.
    #[builder(default)]
    project: Option<String>,
    /// What the task is.
    title: String,
    /// Ids of todos this one depends on, at most <todo-deps-max>.
    #[builder(default)]
    deps: Option<Vec<String>>,
}
