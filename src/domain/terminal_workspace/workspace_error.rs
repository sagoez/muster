use thiserror::Error;

use super::pane_left::PaneLeft;
use crate::domain::value::WorkspacePaneId;

/// Leads why a pane muster could not run in is still open.
const CLOSE_FAILED: &str = ", as closing it failed too:";

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
    #[error(
        "{workspace} opened pane {pane} but could not run muster in it: {cause}; the pane is \
         {left}{}",
        left_because(.left)
    )]
    RunFailed {
        /// The workspace's name.
        workspace: String,
        /// The pane that opened.
        pane: WorkspacePaneId,
        /// Why running in it failed.
        cause: Box<WorkspaceError>,
        /// What became of the pane.
        left: PaneLeft,
    },
}

/// Why `left` is as it is, as a clause after its word: the close's failure
/// for a pane left open, nothing for one closed.
fn left_because(left: &PaneLeft) -> String {
    match left {
        PaneLeft::Closed => String::new(),
        PaneLeft::Open(close) => format!("{CLOSE_FAILED} {close}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A refusal to `action`, as a workspace named `tiles` words it.
    fn refused(action: &str) -> WorkspaceError {
        WorkspaceError::Refused {
            workspace: "tiles".to_string(),
            action: action.to_string(),
            code: "gone".to_string(),
            message: "no pane".to_string(),
        }
    }

    /// The pane's fate reads as its own word, and a pane left open names
    /// why closing it failed.
    #[test]
    fn a_run_failure_says_whether_the_pane_was_closed() {
        let failed = |left| WorkspaceError::RunFailed {
            workspace: "tiles".to_string(),
            pane: WorkspacePaneId::try_new("w1:p2").unwrap(),
            cause: Box::new(refused("run")),
            left,
        };
        assert_eq!(
            failed(PaneLeft::Closed).to_string(),
            "tiles opened pane w1:p2 but could not run muster in it: tiles refused to run: \
             no pane (gone); the pane is closed"
        );
        assert_eq!(
            failed(PaneLeft::Open(Box::new(refused("close")))).to_string(),
            "tiles opened pane w1:p2 but could not run muster in it: tiles refused to run: \
             no pane (gone); the pane is open, as closing it failed too: tiles refused to \
             close: no pane (gone)"
        );
    }
}
