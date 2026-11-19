use getset::Getters;
use serde::Serialize;
use typed_builder::TypedBuilder;

/// A write result `view` naming, the first time a write of this connection
/// answers, the label the agent's writes carry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct Introduced<View> {
    /// The result itself.
    #[serde(flatten)]
    view: View,
    /// The agent's label, on its first write only.
    #[serde(skip_serializing_if = "Option::is_none")]
    you: Option<String>,
}
