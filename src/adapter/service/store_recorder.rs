use std::{
    sync::PoisonError,
    thread,
    time::{Duration, SystemTime},
};

use crossbeam_channel::{Receiver, Sender, bounded, unbounded};

use super::{
    notice::Notice, notice_level::NoticeLevel, shared_notices::SharedNotices,
    store_record::StoreRecord,
};
use crate::domain::{
    config::ConfigError,
    port::{RaisedEdits, RunHistory},
    process::EndedRun,
    value::{EditId, ProcessName, ProjectRoot},
};

/// Longest [`StoreRecorder::flush`] waits for the writes before it to land:
/// past the store's own wait for another writer's lock.
const FLUSH_WAIT: Duration = Duration::from_secs(10);
/// Pause before the first retry of a write that failed.
const FIRST_RETRY: Duration = Duration::from_millis(100);
/// Longest pause between retries of a write that keeps failing.
const MAX_RETRY: Duration = Duration::from_secs(5);
/// Prefix of the notice for a run end that could not be stored yet; the
/// store's error follows.
const RUN_FAILED_NOTICE: &str = "could not record how the process last ended yet, retrying";
/// Prefix of the notice for a raised edit mark that could not be stored yet;
/// the store's error follows.
const RAISED_FAILED_NOTICE: &str = "could not record which agent edits were raised yet, retrying";

/// Stores what the service keeps across restarts, how each process's runs
/// end and which agent edits it raised, on a thread of its own, so a slow or
/// locked database never stalls the loop. Writes land in the order they are
/// sent. One that fails is retried, backing off, until it lands, so a busy
/// database delays a record but never loses it; its first failure becomes a
/// warning notice. Clones share the thread, which ends once every clone is
/// dropped.
#[derive(Clone)]
pub struct StoreRecorder {
    /// Where writes go to the thread.
    sender: Sender<StoreRecord>,
}

impl StoreRecorder {
    /// Starts the thread writing run ends to `runs` and raised edits to
    /// `raised`, telling `notices` of a write that has to wait.
    pub fn spawn(
        runs: Box<dyn RunHistory>,
        raised: Box<dyn RaisedEdits>,
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
/// retrying one that fails until it lands and telling `notices` the first
/// time it fails.
fn write_all(
    records: &Receiver<StoreRecord>,
    runs: &dyn RunHistory,
    raised: &dyn RaisedEdits,
    notices: &SharedNotices,
) {
    for record in records {
        if let StoreRecord::Flush(done) = record {
            let _ = done.send(());
            continue;
        }
        let mut pause = FIRST_RETRY;
        let mut told = false;
        while let Err(error) = apply(&record, runs, raised) {
            if !told {
                tell(notices, &record, &error);
                told = true;
            }
            thread::sleep(pause);
            pause = pause.saturating_mul(2).min(MAX_RETRY);
        }
    }
}

/// Writes `record` to the store it belongs to.
///
/// # Errors
/// Returns the store's error when the write fails.
fn apply(
    record: &StoreRecord,
    runs: &dyn RunHistory,
    raised: &dyn RaisedEdits,
) -> Result<(), ConfigError> {
    match record {
        StoreRecord::Ended { project, run } => runs.record_end(project, run),
        StoreRecord::Cleared { project, process } => runs.clear_end(project, process),
        StoreRecord::EditsRaised { project, through } => raised.mark_raised(project, *through),
        StoreRecord::Flush(_) => Ok(()),
    }
}

/// Tells `notices` that `record` failed with `error` and will be retried.
fn tell(notices: &SharedNotices, record: &StoreRecord, error: &ConfigError) {
    let (project, process, failed) = match record {
        StoreRecord::Ended { project, run } => {
            (project, Some(run.process().clone()), RUN_FAILED_NOTICE)
        },
        StoreRecord::Cleared { project, process } => {
            (project, Some(process.clone()), RUN_FAILED_NOTICE)
        },
        StoreRecord::EditsRaised { project, .. } => (project, None, RAISED_FAILED_NOTICE),
        StoreRecord::Flush(_) => return,
    };
    notices.lock().unwrap_or_else(PoisonError::into_inner).push(
        Notice::builder()
            .at(SystemTime::now())
            .project(Some(project.clone()))
            .process(process)
            .level(NoticeLevel::Warning)
            .message(format!("{failed}: {error}"))
            .build(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        adapter::service::{failing_history::FailingHistory, notices::Notices},
        domain::{pty::ExitOutcome, value::ProcessName},
    };

    /// The project every record here is of.
    fn shop() -> ProjectRoot {
        ProjectRoot::try_new("/shop".into()).unwrap()
    }

    /// A run of `api` that ended just now with status 0.
    fn run() -> EndedRun {
        EndedRun::builder()
            .process(ProcessName::try_new("api").unwrap())
            .outcome(ExitOutcome::Succeeded)
            .at(SystemTime::now())
            .build()
    }

    /// A write the store refuses for a while is retried until it lands,
    /// never dropped, and its failure is told once, not once per attempt.
    #[test]
    fn a_failed_write_is_retried_until_it_lands_and_told_once() {
        let history = FailingHistory::failing(3);
        let notices = Notices::shared(None);
        let recorder = StoreRecorder::spawn(
            Box::new(history.clone()),
            Box::new(history.clone()),
            notices.clone(),
        );

        recorder.ended(&shop(), run());
        recorder.flush();

        assert_eq!(history.recorded(), [ProcessName::try_new("api").unwrap()]);
        assert_eq!(history.attempts(), 4);
        let told = notices.lock().unwrap().recent(&shop());
        assert_eq!(told.len(), 1, "{told:?}");
        assert!(told[0].message().starts_with(RUN_FAILED_NOTICE), "{told:?}");
    }
}
