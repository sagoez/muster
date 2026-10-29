use getset::Getters;
use schemars::JsonSchema;
use serde::Deserialize;
use typed_builder::TypedBuilder;

/// Key of a scratchpad or key-value entry.
#[derive(Debug, Deserialize, JsonSchema, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct KeyArg {
    /// The entry's key.
    key: String,
}
