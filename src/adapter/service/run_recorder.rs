use std::{
    sync::{Arc, PoisonError},
    thread,
    time::{Duration, SystemTime},
};

use crossbeam_channel::{Receiver, Sender, bounded, unbounded};

use super::{
    notice::Notice, notice_level::NoticeLevel, run_record::RunRecord, shared_notices::SharedNotices,
};
use crate::domain::{
    port::RunHistory,
    process::EndedRun,
    value::{ProcessName, ProjectRoot},
};

/// Longest [`RunRecorder::flush`] waits for the writes before it to land:
/// past the store's own wait for another writer's lock.
const FLUSH_WAIT: Duration = Duration::from_secs(10);
/// Prefix of the notice for a run end that could not be stored; the store's
/// error follows.
const RECORD_FAILED_NOTICE: &str = "could not record how the process last ended";

/// Stores how each process's runs end on a thread of its own, so a slow or
/// locked database never stalls the loop. Writes land in the order they are
/// sent; one that fails becomes a warning notice. Clones share the thread,
/// which ends once every clone is dropped.
#[derive(Clone)]
pub struct RunRecorder {
    /// Where writes go to the thread.
    sender: Sender<RunRecord>,
}

impl RunRecorder {
    /// Starts the thread writing to `history`, telling `notices` of a failed
    /// write.
    pub fn spawn(history: Arc<dyn RunHistory>, notices: SharedNotices) -> Self {
        let (sender, records) = unbounded();
        thread::spawn(move || write_all(&records, history.as_ref(), &notices));
        Self { sender }
    }

    /// Records that `run` of `project` ended on its own.
    pub fn ended(&self, project: &ProjectRoot, run: EndedRun) {
        let _ = self.sender.send(RunRecord::Ended {
            project: project.clone(),
            run,
        });
    }

    /// Forgets how `process` of `project` last ended.
    pub fn cleared(&self, project: &ProjectRoot, process: &ProcessName) {
        let _ = self.sender.send(RunRecord::Cleared {
            project: project.clone(),
            process: process.clone(),
        });
    }

    /// Waits, at most [`FLUSH_WAIT`], until every write sent before it landed,
    /// so a service exiting leaves its last run ends stored.
    pub fn flush(&self) {
        let (done, landed) = bounded(1);
        if self.sender.send(RunRecord::Flush(done)).is_ok() {
            let _ = landed.recv_timeout(FLUSH_WAIT);
        }
    }
}

/// Applies each record from `records` to `history` in order until every
/// sender is gone, telling `notices` of a write that failed.
fn write_all(records: &Receiver<RunRecord>, history: &dyn RunHistory, notices: &SharedNotices) {
    for record in records {
        let (project, process, written) = match record {
            RunRecord::Ended { project, run } => {
                let written = history.record_end(&project, &run);
                (project, run.process().clone(), written)
            },
            RunRecord::Cleared { project, process } => {
                let written = history.clear_end(&project, &process);
                (project, process, written)
            },
            RunRecord::Flush(done) => {
                let _ = done.send(());
                continue;
            },
        };
        if let Err(error) = written {
            notices.lock().unwrap_or_else(PoisonError::into_inner).push(
                Notice::builder()
                    .at(SystemTime::now())
                    .project(Some(project))
                    .process(Some(process))
                    .level(NoticeLevel::Warning)
                    .message(format!("{RECORD_FAILED_NOTICE}: {error}"))
                    .build(),
            );
        }
    }
}
