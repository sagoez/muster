use crate::domain::value::PaneId;

/// The workspace row an opened agent session landed on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionRow {
    /// The session already had a row, which was reused.
    Existing(PaneId),
    /// A new row was inserted for the session.
    Inserted(PaneId),
}

impl SessionRow {
    /// The row's pane, however it was reached.
    pub fn pane(self) -> PaneId {
        match self {
            Self::Existing(pane) | Self::Inserted(pane) => pane,
        }
    }
}
