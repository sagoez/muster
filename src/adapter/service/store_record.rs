use crossbeam_channel::Sender;

use crate::domain::{
    process::EndedRun,
    value::{EditId, ProcessName, ProjectRoot},
};

/// One write a [`StoreRecorder`](super::store_recorder::StoreRecorder)
/// applies to the store, in the order it was sent.
pub enum StoreRecord {
    /// A process's run ended on its own.
    Ended {
        /// The process's project.
        project: ProjectRoot,
        /// How the run ended.
        run: EndedRun,
    },
    /// A process runs again, or was stopped on request.
    Cleared {
        /// The process's project.
        project: ProjectRoot,
        /// The process.
        process: ProcessName,
    },
    /// The agent edits of a project were raised as events.
    EditsRaised {
        /// The project.
        project: ProjectRoot,
        /// The newest edit raised.
        through: EditId,
    },
    /// Answers once every earlier write was applied.
    Flush(Sender<()>),
}
