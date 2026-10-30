use getset::{CopyGetters, Getters};
use serde::Serialize;
use typed_builder::TypedBuilder;

use crate::domain::value::{ProcessName, ProjectName, WorkspacePaneId};

/// A process opened in a new pane, as `--json` and MCP show it:
/// `{"project": "shop", "process": "web", "pane": "w1:p4", "read_only": false}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Getters, CopyGetters, TypedBuilder)]
pub struct OpenView {
    /// The project's name.
    #[getset(get = "pub")]
    project: ProjectName,
    /// The process shown.
    #[getset(get = "pub")]
    process: ProcessName,
    /// The pane it is shown in.
    #[getset(get = "pub")]
    pane: WorkspacePaneId,
    /// Whether the pane only watches it.
    #[getset(get_copy = "pub")]
    read_only: bool,
}
