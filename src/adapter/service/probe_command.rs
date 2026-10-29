use super::probe_task::ProbeTask;
use crate::domain::value::{PaneId, ProjectRoot};

/// What the loop asks of the probe worker.
pub(super) enum ProbeCommand {
    /// Try a probe until it passes or its time runs out.
    Watch(ProbeTask),
    /// Give up every probe of a pane.
    Forget {
        /// The pane's project.
        project: ProjectRoot,
        /// The pane.
        pane: PaneId,
    },
}
