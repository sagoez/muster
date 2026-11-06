use getset::{CopyGetters, Getters};
use schemars::JsonSchema;
use serde::Deserialize;
use typed_builder::TypedBuilder;

/// The change `todo_update` makes to one todo.
#[derive(Debug, Deserialize, JsonSchema, Getters, CopyGetters, TypedBuilder)]
#[serde(deny_unknown_fields)]
pub struct TodoUpdateArg {
    /// A registered project's name or a folder; defaults to the project
    /// containing the server's working directory.
    #[builder(default)]
    #[getset(get = "pub")]
    project: Option<String>,
    /// The todo's id.
    #[getset(get = "pub")]
    id: String,
    /// Its new status: pending, in-progress or done.
    #[builder(default)]
    #[getset(get = "pub")]
    status: Option<String>,
    /// Its new title.
    #[builder(default)]
    #[getset(get = "pub")]
    title: Option<String>,
    /// Hand it back as pending, clearing its owner.
    #[builder(default)]
    #[getset(get_copy = "pub")]
    release: Option<bool>,
    /// Change it even though another agent holds it.
    #[builder(default)]
    #[getset(get_copy = "pub")]
    force: Option<bool>,
}
