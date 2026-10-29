use serde::{Deserialize, Serialize};

use super::refusal::Refusal;
use crate::domain::value::{ProcessName, ProjectName};

/// What the service tells a shell's hook to print at its next prompt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ShellNote {
    /// Entering the project started these processes.
    Started {
        /// The project entered.
        project: ProjectName,
        /// What started, in stack order.
        names: Vec<ProcessName>,
    },
    /// A process the project's entering would start was refused.
    Refused {
        /// The project entered.
        project: ProjectName,
        /// Which process, and why.
        refusal: Refusal,
    },
}
