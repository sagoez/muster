use getset::{CopyGetters, Getters};
use schemars::JsonSchema;
use serde::Deserialize;
use typed_builder::TypedBuilder;

/// Which todos `todo_list` returns.
#[derive(Debug, Default, Deserialize, JsonSchema, Getters, CopyGetters, TypedBuilder)]
#[serde(deny_unknown_fields)]
pub struct TodoListArg {
    /// A registered project's name or a folder; defaults to the project
    /// containing the server's working directory.
    #[builder(default)]
    #[getset(get = "pub")]
    project: Option<String>,
    /// Only todos in this status: pending, in-progress or done.
    #[builder(default)]
    #[getset(get = "pub")]
    status: Option<String>,
    /// Only todos this agent holds: "me", or another agent's label.
    #[builder(default)]
    #[getset(get = "pub")]
    owner: Option<String>,
    /// Only pending todos whose dependencies are all done.
    #[builder(default)]
    #[getset(get_copy = "pub")]
    claimable: Option<bool>,
    /// Most todos returned, default 50, at most 200.
    #[builder(default)]
    #[getset(get_copy = "pub")]
    limit: Option<usize>,
}
