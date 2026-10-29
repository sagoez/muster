use std::{
    sync::{Arc, PoisonError},
    thread,
    time::{Duration, SystemTime},
};

use crossbeam_channel::{Receiver, Sender, bounded, unbounded};

use super::{
    notice::Notice, notice_level::NoticeLevel, shared_notices::SharedNotices,
    store_record::StoreRecord,
};
use crate::domain::{
    port::{RaisedEdits, RunHistory},
    process::EndedRun,
    value::{EditId, ProcessName, ProjectRoot},
};

/// Longest [`StoreRecorder::flush`] waits for the writes before it to land:
/// past the store's own wait for another writer's lock.
const FLUSH_WAIT: Duration = Duration::from_secs(10);
/// Prefix of the notice for a run end that could not be stored; the store's
/// error follows.
const RUN_FAILED_NOTICE: &str = "could not record how the process last ended";
/// Prefix of the notice for a raised edit mark that could not be stored; the
/// store's error follows.
const RAISED_FAILED_NOTICE: &str = "could not record which agent edits were raised";

/// Stores what the service keeps across restarts, how each process's runs
/// end and which agent edits it raised, on a thread of its own, so a slow or
/// locked database never stalls the loop. Writes land in the order they are
/// sent; one that fails becomes a warning notice. Clones share the thread,
/// which ends once every clone is dropped.
#[derive(Clone)]
pub struct StoreRecorder {
    /// Where writes go to the thread.
    sender: Sender<StoreRecord>,
}

impl StoreRecorder {
    /// Starts the thread writing run ends to `runs` and raised edits to
    /// `raised`, telling `notices` of a failed write.
    pub fn spawn(
        runs: Arc<dyn RunHistory>,
        raised: Arc<dyn RaisedEdits>,
        notices: SharedNotices,
    ) -> Self {
        let (sender, records) = unbounded();
        thread::spawn(move || write_all(&records, runs.as_ref(), raised.as_ref(), &notices));
        Self { sender }
    }

    /// Records that `run` of `project` ended on its own.
    pub fn ended(&self, project: &ProjectRoot, run: EndedRun) {
        let _ = self.sender.send(StoreRecord::Ended {
            project: project.clone(),
            run,
        });
    }

    /// Forgets how `process` of `project` last ended.
    pub fn cleared(&self, project: &ProjectRoot, process: &ProcessName) {
        let _ = self.sender.send(StoreRecord::Cleared {
            project: project.clone(),
            process: process.clone(),
        });
    }

    /// Records that every agent edit of `project` up to `through` was raised.
    pub fn edits_raised(&self, project: &ProjectRoot, through: EditId) {
        let _ = self.sender.send(StoreRecord::EditsRaised {
            project: project.clone(),
            through,
        });
    }

    /// Waits, at most `FLUSH_WAIT`, until every write sent before it landed,
    /// so a service exiting leaves its last records stored.
    pub fn flush(&self) {
        let (done, landed) = bounded(1);
        if self.sender.send(StoreRecord::Flush(done)).is_ok() {
            let _ = landed.recv_timeout(FLUSH_WAIT);
        }
    }
}

/// Applies each record from `records` in order until every sender is gone,
/// telling `notices` of a write that failed.
fn write_all(
    records: &Receiver<StoreRecord>,
    runs: &dyn RunHistory,
    raised: &dyn RaisedEdits,
    notices: &SharedNotices,
) {
    for record in records {
        let (project, process, failed, written) = match record {
            StoreRecord::Ended { project, run } => {
                let written = runs.record_end(&project, &run);
                (
                    project,
                    Some(run.process().clone()),
                    RUN_FAILED_NOTICE,
                    written,
                )
            },
            StoreRecord::Cleared { project, process } => {
                let written = runs.clear_end(&project, &process);
                (project, Some(process), RUN_FAILED_NOTICE, written)
            },
            StoreRecord::EditsRaised { project, through } => {
                let written = raised.mark_raised(&project, through);
                (project, None, RAISED_FAILED_NOTICE, written)
            },
            StoreRecord::Flush(done) => {
                let _ = done.send(());
                continue;
            },
        };
        if let Err(error) = written {
            notices.lock().unwrap_or_else(PoisonError::into_inner).push(
                Notice::builder()
                    .at(SystemTime::now())
                    .project(Some(project))
                    .process(process)
                    .level(NoticeLevel::Warning)
                    .message(format!("{failed}: {error}"))
                    .build(),
            );
        }
    }
}
