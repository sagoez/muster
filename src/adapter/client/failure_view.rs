use getset::{CopyGetters, Getters};
use serde::Serialize;
use serde_json::{Value, json};
use typed_builder::TypedBuilder;

use super::failure_kind::FailureKind;

/// A failure as JSON reports it, so a script or an agent can act on it
/// without parsing the message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Getters, CopyGetters, TypedBuilder)]
pub struct FailureView {
    /// Why, as a kind.
    #[getset(get_copy = "pub")]
    kind: FailureKind,
    /// Why, for a person or an agent to read.
    #[getset(get = "pub")]
    message: String,
    /// The names the caller could have meant: processes, projects, todo ids
    /// or keys.
    #[builder(setter(transform = |names: Vec<impl ToString>| names.iter().map(ToString::to_string).collect()))]
    #[getset(get = "pub")]
    candidates: Vec<String>,
}

impl FailureView {
    /// The failure as the JSON object it is reported as: `{"error": {...}}`.
    pub fn to_json(&self) -> Value {
        json!({ "error": self })
    }
}
