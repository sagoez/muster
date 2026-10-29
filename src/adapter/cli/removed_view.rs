use getset::{CopyGetters, Getters};
use serde::Serialize;
use typed_builder::TypedBuilder;

use crate::domain::value::ProcessName;

/// One process `muster rm` or `muster import --replace` removed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Getters, CopyGetters, TypedBuilder)]
pub struct RemovedView {
    /// The process.
    #[getset(get = "pub")]
    name: ProcessName,
    /// Whether it was running and was stopped.
    #[getset(get_copy = "pub")]
    stopped: bool,
}
