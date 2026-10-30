use getset::Getters;
use typed_builder::TypedBuilder;

use crate::domain::value::WorkspacePaneId;

/// A terminal workspace that answered for the caller's pane, as `doctor`
/// shows it.
#[derive(Clone, Debug, PartialEq, Eq, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct WorkspaceProbe {
    /// The workspace's name.
    workspace: String,
    /// The caller's pane.
    pane: WorkspacePaneId,
}
