use getset::Getters;
use schemars::JsonSchema;
use serde::Deserialize;
use typed_builder::TypedBuilder;

/// The setting `config_get` reads, or none for all of them.
#[derive(Debug, Deserialize, JsonSchema, Getters, TypedBuilder)]
#[serde(deny_unknown_fields)]
#[getset(get = "pub")]
pub struct ConfigGetArg {
    /// The setting's key, such as `default-restart`. Leave it out to read
    /// every setting.
    #[builder(default)]
    key: Option<String>,
}
