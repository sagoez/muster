use getset::Getters;
use schemars::JsonSchema;
use serde::Deserialize;
use typed_builder::TypedBuilder;

/// A key-value key and its value.
#[derive(Debug, Deserialize, JsonSchema, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct KvSetArg {
    /// The entry's key.
    key: String,
    /// The value to store.
    value: String,
}
