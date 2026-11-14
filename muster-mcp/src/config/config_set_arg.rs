use getset::Getters;
use schemars::JsonSchema;
use serde::Deserialize;
use typed_builder::TypedBuilder;

/// A setting and the text of its new value.
#[derive(Debug, Deserialize, JsonSchema, Getters, TypedBuilder)]
#[serde(deny_unknown_fields)]
#[getset(get = "pub")]
pub struct ConfigSetArg {
    /// The setting's key, such as `default-restart`.
    key: String,
    /// The new value, written as `config_get` shows it, such as `on-failure`,
    /// `24x80`, `10MiB` or `5s`.
    value: String,
}
