use thiserror::Error;

use crate::domain::value::WorkspacePaneId;

/// Why a terminal workspace did not open or answer for a pane. Every text
/// it carries from the workspace is already made safe to print.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum WorkspaceError {
    /// The workspace could not be reached: it is missing, would not start,
    /// or did not answer in time.
    #[error("cannot reach {workspace}: {reason}")]
    Unreachable {
        /// The workspace's name.
        workspace: String,
        /// What went wrong.
        reason: String,
    },
    /// The workspace refused what was asked.
    #[error("{workspace} refused to {action}: {message} ({code})")]
    Refused {
        /// The workspace's name.
        workspace: String,
        /// What was asked of it.
        action: String,
        /// The workspace's code for the refusal.
        code: String,
        /// The workspace's own words.
        message: String,
    },
    /// The workspace answered with something muster cannot read.
    #[error("{workspace} answered {action} with something muster cannot read: {reason}")]
    Unreadable {
        /// The workspace's name.
        workspace: String,
        /// What was asked of it.
        action: String,
        /// Why the answer could not be read.
        reason: String,
    },
    /// A pane opened but muster could not be run in it.
    #[error("{workspace} opened pane {pane} but could not run muster in it: {cause}{closed}")]
    RunFailed {
        /// The workspace's name.
        workspace: String,
        /// The pane that opened.
        pane: WorkspacePaneId,
        /// Why running in it failed.
        cause: Box<WorkspaceError>,
        /// What became of the pane, as a clause to append.
        closed: String,
    },
}
