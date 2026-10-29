use getset::Getters;
use serde::{Deserialize, Serialize};
use typed_builder::TypedBuilder;

use super::service_error_kind::ServiceErrorKind;
use crate::domain::value::ProcessName;

/// A refused request: why, in words for a person and as a kind for a client,
/// with the valid process names when the name was unknown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Getters, TypedBuilder)]
#[getset(get = "pub")]
#[serde(rename_all = "kebab-case")]
pub struct ServiceErrorBody {
    /// What kind of refusal it is.
    kind: ServiceErrorKind,
    /// What went wrong, for a person to read.
    message: String,
    /// The project's process names, when the request named an unknown one.
    #[builder(default)]
    candidates: Vec<ProcessName>,
}
