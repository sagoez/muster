use crossbeam_channel::Sender;

use crate::domain::{
    process::EndedRun,
    value::{ProcessName, ProjectRoot},
};

/// One write a [`RunRecorder`](super::run_recorder::RunRecorder) applies to
/// the store, in the order it was sent.
pub enum RunRecord {
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
    /// Answers once every earlier write was applied.
    Flush(Sender<()>),
}
