use getset::Getters;
use schemars::JsonSchema;
use serde::Deserialize;
use typed_builder::TypedBuilder;

/// The folder `project_register` registers.
#[derive(Debug, Default, Deserialize, JsonSchema, Getters, TypedBuilder)]
#[serde(deny_unknown_fields)]
#[getset(get = "pub")]
pub struct ProjectRegisterArg {
    /// The folder to register, this server's folder or one inside it, read
    /// against the server's working directory; defaults to this server's
    /// folder.
    #[builder(default)]
    project: Option<String>,
}
