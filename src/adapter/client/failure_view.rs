use getset::{CopyGetters, Getters};
use serde::Serialize;
use serde_json::{Value, json};
use typed_builder::TypedBuilder;

use super::failure_kind::FailureKind;
use crate::domain::value::ProcessName;

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
    /// The process names the caller could have meant.
    #[getset(get = "pub")]
    candidates: Vec<ProcessName>,
}

impl FailureView {
    /// The failure as the JSON object it is reported as: `{"error": {...}}`.
    pub fn to_json(&self) -> Value {
        json!({ "error": self })
    }
}
