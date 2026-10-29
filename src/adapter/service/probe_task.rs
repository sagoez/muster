use std::time::Instant;

use super::{probe_target::ProbeTarget, supervisor::SpawnGeneration};
use crate::domain::value::{PaneId, ProjectRoot};

/// One probe the worker tries until it passes or its time runs out.
pub(super) struct ProbeTask {
    /// The project of the process it is for.
    pub(super) project: ProjectRoot,
    /// The pane of the process it is for.
    pub(super) pane: PaneId,
    /// The run it is for.
    pub(super) generation: SpawnGeneration,
    /// What it tries.
    pub(super) target: ProbeTarget,
    /// When it is given up.
    pub(super) until: Instant,
}
