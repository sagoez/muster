use getset::{CopyGetters, Getters};
use muster_client::{DefinitionView, ProjectIdentityView};
use serde::Serialize;
use typed_builder::TypedBuilder;

/// What `process_set` answers: the process as stored now, whether the call
/// changed it, and whether it runs.
#[derive(Debug, Clone, PartialEq, Serialize, Getters, CopyGetters, TypedBuilder)]
pub struct ChangedView {
    /// The project the process belongs to.
    #[getset(get = "pub")]
    project: ProjectIdentityView,
    /// The stored definition, its environment as variable names only.
    #[getset(get = "pub")]
    process: DefinitionView,
    /// Whether any field took a new value.
    #[getset(get_copy = "pub")]
    changed: bool,
    /// Whether the process runs, keeping its current run until restarted.
    #[getset(get_copy = "pub")]
    running: bool,
}
