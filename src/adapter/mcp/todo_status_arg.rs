use getset::Getters;
use schemars::JsonSchema;
use serde::Deserialize;
use typed_builder::TypedBuilder;

/// A todo id and its new status.
#[derive(Debug, Deserialize, JsonSchema, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct TodoStatusArg {
    /// The todo's id.
    id: String,
    /// New status: `pending`, `in-progress`, or `done`.
    status: String,
}
