use getset::Getters;
use schemars::JsonSchema;
use serde::Deserialize;
use typed_builder::TypedBuilder;

/// A scratchpad key and its markdown body.
#[derive(Debug, Deserialize, JsonSchema, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct ScratchpadSetArg {
    /// The note's key.
    key: String,
    /// The note's markdown body.
    body: String,
}
