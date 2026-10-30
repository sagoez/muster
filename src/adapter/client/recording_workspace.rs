use std::sync::Mutex;

use crate::domain::{
    port::TerminalWorkspace,
    terminal_workspace::{OpenedPane, PaneRequest, SplitDirection, WorkspaceError, WorkspaceProbe},
    value::WorkspacePaneId,
};

/// The name a [`RecordingWorkspace`] answers a check with.
pub(crate) const RECORDING_WORKSPACE: &str = "recording";
/// The caller's pane a [`RecordingWorkspace`] answers a check with.
pub(crate) const CALLER_PANE: &str = "w1:p1";

/// A [`TerminalWorkspace`] that records every pane asked of it and answers
/// each ask and check as the test scripted, opening nothing.
pub(crate) struct RecordingWorkspace {
    /// Every pane asked for, in order.
    requests: Mutex<Vec<PaneRequest>>,
    /// The pane each ask opens, or the failure every ask and check meets.
    answer: Result<WorkspacePaneId, WorkspaceError>,
}

impl RecordingWorkspace {
    /// A workspace that opens the pane `pane` for every ask, split as asked
    /// or right.
    pub(crate) fn opening(pane: &str) -> Self {
        Self {
            requests: Mutex::default(),
            answer: Ok(WorkspacePaneId::try_new(pane).unwrap()),
        }
    }

    /// A workspace that fails every ask and check with `error`.
    pub(crate) fn failing(error: WorkspaceError) -> Self {
        Self {
            requests: Mutex::default(),
            answer: Err(error),
        }
    }

    /// Every pane asked for, in order.
    pub(crate) fn requests(&self) -> Vec<PaneRequest> {
        self.requests.lock().unwrap().clone()
    }
}

impl TerminalWorkspace for RecordingWorkspace {
    fn open_beside(&self, request: &PaneRequest) -> Result<OpenedPane, WorkspaceError> {
        self.requests.lock().unwrap().push(request.clone());
        let pane = self.answer.clone()?;
        Ok(OpenedPane::builder()
            .pane(pane)
            .direction(request.direction().unwrap_or(SplitDirection::Right))
            .build())
    }

    fn check(&self) -> Result<WorkspaceProbe, WorkspaceError> {
        self.answer.clone()?;
        Ok(WorkspaceProbe::builder()
            .workspace(RECORDING_WORKSPACE.to_string())
            .pane(WorkspacePaneId::try_new(CALLER_PANE).unwrap())
            .build())
    }
}
