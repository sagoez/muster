use serde::{Deserialize, Serialize};

use super::{ExitCauseWire, WireForm};
use crate::domain::pty::ExitOutcome;

/// How a process ended, as the holder and service carry it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub enum ExitOutcomeWire {
    /// Exited with status zero.
    Succeeded,
    /// Exited with a non-zero status or was ended by a signal.
    Failed(ExitCauseWire),
}

impl WireForm for ExitOutcome {
    type Wire = ExitOutcomeWire;

    fn to_wire(&self) -> ExitOutcomeWire {
        match self {
            Self::Succeeded => ExitOutcomeWire::Succeeded,
            Self::Failed(cause) => ExitOutcomeWire::Failed(cause.into()),
        }
    }

    fn from_wire(wire: ExitOutcomeWire) -> Result<Self, String> {
        Ok(match wire {
            ExitOutcomeWire::Succeeded => Self::Succeeded,
            ExitOutcomeWire::Failed(cause) => Self::Failed(cause.into()),
        })
    }
}
