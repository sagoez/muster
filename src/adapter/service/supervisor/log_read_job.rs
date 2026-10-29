use super::{adopter::HolderAdopter, log_ask::LogAsk, log_read_result::LogReadResult};
use crate::domain::value::{ProcessName, ProjectRoot};

/// What one read of a process's log off the loop is asked: its end, for
/// waits on a process this service never saw run, and what each held wait
/// or answer needs, by its key.
pub(super) struct LogReadJob {
    /// Whether the log's end is read.
    pub(super) end: bool,
    /// Each thing asked, by the key of the held wait or answer asking it.
    pub(super) asks: Vec<(u64, LogAsk)>,
}

impl LogReadJob {
    /// Reads all of it from the log of the process `name` of `project`
    /// through `adopter`.
    pub(super) fn run(
        self,
        adopter: &dyn HolderAdopter,
        project: &ProjectRoot,
        name: &ProcessName,
    ) -> LogReadResult {
        let tail = if self.end {
            adopter.log_tail(project, name)
        } else {
            None
        };
        let answers = self
            .asks
            .into_iter()
            .filter_map(|(key, ask)| {
                ask.answer(adopter, project, name)
                    .map(|answer| (key, answer))
            })
            .collect();
        LogReadResult::builder().tail(tail).answers(answers).build()
    }
}
