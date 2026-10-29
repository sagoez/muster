use serde::Serialize;

use crate::{
    adapter::service::PortOwner,
    domain::value::{Pid, ProcessName, ProjectName},
};

/// Who holds a port a start needs, as `--json` shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum PortOwnerView {
    /// A process muster supervises.
    Supervised {
        /// Its project.
        project: ProjectName,
        /// Its name in that project.
        process: ProcessName,
    },
    /// A process muster does not supervise.
    Other {
        /// Its process id.
        pid: Pid,
        /// Its command line, or its executable when that is unknown.
        command: String,
    },
}

impl From<&PortOwner> for PortOwnerView {
    fn from(owner: &PortOwner) -> Self {
        match owner {
            PortOwner::Supervised(holder) => Self::Supervised {
                project: holder.project().clone(),
                process: holder.process().clone(),
            },
            PortOwner::Other(occupant) => Self::Other {
                pid: occupant.pid(),
                command: occupant.command().clone(),
            },
        }
    }
}
