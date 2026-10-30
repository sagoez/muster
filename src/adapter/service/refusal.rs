use getset::Getters;
use serde::{Deserialize, Serialize};
use typed_builder::TypedBuilder;

use crate::domain::value::ProcessName;

/// One process a project-wide request left alone, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Getters, TypedBuilder)]
#[getset(get = "pub")]
#[serde(rename_all = "kebab-case")]
pub struct Refusal {
    /// The process.
    name: ProcessName,
    /// Why it was left alone, for a person to read.
    reason: String,
}
