use getset::CopyGetters;
use schemars::JsonSchema;
use serde::Deserialize;
use typed_builder::TypedBuilder;

/// How many entries a list tool should return.
#[derive(Debug, Deserialize, JsonSchema, CopyGetters, TypedBuilder)]
#[getset(get_copy = "pub")]
pub struct ListArg {
    /// Maximum entries to return. Defaults to 50, capped at 200.
    #[serde(default)]
    #[builder(default)]
    limit: Option<usize>,
}
