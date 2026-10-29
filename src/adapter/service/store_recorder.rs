use std::{
    sync::PoisonError,
    thread,
    time::{Duration, SystemTime},
};

use crossbeam_channel::{Receiver, Sender, bounded, unbounded};

use super::{
    notice::Notice, notice_level::NoticeLevel, record_retry::RecordRetry,
    shared_notices::SharedNotices, store_record::StoreRecord,
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
/// Prefix of the notice for a run end that could not be stored yet; the
/// store's error follows.
const RUN_FAILED_NOTICE: &str = "could not record how the process last ended yet, retrying";
/// Prefix of the notice for a raised edit mark that could not be stored yet;
/// the store's error follows.
const RAISED_FAILED_NOTICE: &str = "could not record which agent edits were raised yet, retrying";
/// Prefix of the notice for a record given up; what it was and the store's
/// last error follow.
const GAVE_UP_NOTICE: &str = "gave up recording";
/// What a given-up run end was, before the process.
const ENDED_RECORD: &str = "how a run ended for";
/// What a given-up cleared run end was, before the process.
const CLEARED_RECORD: &str = "that a run ended on request for";
/// What a given-up raised edit mark was, before the newest edit.
const RAISED_RECORD: &str = "agent edits raised through";

/// Stores what the service keeps across restarts, how each process's runs
/// end and which agent edits it raised, on a thread of its own, so a slow or
/// locked database never stalls the loop. Writes land in the order they are
/// sent. One that fails is retried under a [`RecordRetry`], so a busy
/// database delays a record; its first failure becomes a warning notice. One
/// that still fails after the last try is given up with an error notice
/// naming it, so a database that stays broken never holds back the records
/// after it. Clones share the thread, which ends once every clone is dropped.
#[derive(Clone)]
pub struct StoreRecorder {
    /// Where writes go to the thread.
    sender: Sender<StoreRecord>,
}

impl StoreRecorder {
    /// Starts the thread writing run ends to `runs` and raised edits to
    /// `raised` under `retry`, telling `notices` of a write that has to wait
    /// or was given up.
    pub fn spawn(
        runs: Box<dyn RunHistory>,
        raised: Box<dyn RaisedEdits>,
        notices: SharedNotices,
        retry: RecordRetry,
    ) -> Self {
        let (sender, records) = unbounded();
        thread::spawn(move || {
            write_all(&records, runs.as_ref(), raised.as_ref(), &notices, retry);
        });
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
/// retrying one that fails as `retry` says, telling `notices` the first time
/// it fails and again when it is given up.
fn write_all(
    records: &Receiver<StoreRecord>,
    runs: &dyn RunHistory,
    raised: &dyn RaisedEdits,
    notices: &SharedNotices,
    retry: RecordRetry,
) {
    for record in records {
        if let StoreRecord::Flush(done) = record {
            let _ = done.send(());
            continue;
        }
        let mut pause = retry.first();
        let mut tries = 1;
        while let Err(error) = apply(&record, runs, raised) {
            if tries >= retry.attempts() {
                give_up(notices, &record, tries, &error);
                break;
            }
            if tries == 1 {
                tell(notices, &record, &error);
            }
            thread::sleep(pause);
            pause = pause.saturating_mul(2).min(retry.max());
            tries += 1;
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
    let failed = match record {
        StoreRecord::Ended { .. } | StoreRecord::Cleared { .. } => RUN_FAILED_NOTICE,
        StoreRecord::EditsRaised { .. } => RAISED_FAILED_NOTICE,
        StoreRecord::Flush(_) => return,
    };
    push(
        notices,
        record,
        NoticeLevel::Warning,
        format!("{failed}: {error}"),
    );
}

/// Tells `notices`, as an error, that `record` was given up after `tries`
/// tries, the last failing with `error`.
fn give_up(notices: &SharedNotices, record: &StoreRecord, tries: u32, error: &ConfigError) {
    let what = match record {
        StoreRecord::Ended { run, .. } => format!("{ENDED_RECORD} {}", run.process()),
        StoreRecord::Cleared { process, .. } => format!("{CLEARED_RECORD} {process}"),
        StoreRecord::EditsRaised { through, .. } => format!("{RAISED_RECORD} {through}"),
        StoreRecord::Flush(_) => return,
    };
    push(
        notices,
        record,
        NoticeLevel::Error,
        format!("{GAVE_UP_NOTICE} {what} after {tries} tries: {error}"),
    );
}

/// Pushes `message` at `level` onto `notices`, about the project and process
/// `record` is of.
fn push(notices: &SharedNotices, record: &StoreRecord, level: NoticeLevel, message: String) {
    let (project, process) = match record {
        StoreRecord::Ended { project, run } => (project, Some(run.process().clone())),
        StoreRecord::Cleared { project, process } => (project, Some(process.clone())),
        StoreRecord::EditsRaised { project, .. } => (project, None),
        StoreRecord::Flush(_) => return,
    };
    notices.lock().unwrap_or_else(PoisonError::into_inner).push(
        Notice::builder()
            .at(SystemTime::now())
            .project(Some(project.clone()))
            .process(process)
            .level(level)
            .message(message)
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

    /// Tries a record gets under [`quick`].
    const QUICK_ATTEMPTS: u32 = 5;

    /// A retry that backs off in milliseconds, so a test gives up quickly.
    fn quick() -> RecordRetry {
        RecordRetry::builder()
            .first(Duration::from_millis(1))
            .max(Duration::from_millis(4))
            .attempts(QUICK_ATTEMPTS)
            .build()
    }

    /// A recorder over `history` telling `notices`, retrying as [`quick`].
    fn recorder(history: &FailingHistory, notices: &SharedNotices) -> StoreRecorder {
        StoreRecorder::spawn(
            Box::new(history.clone()),
            Box::new(history.clone()),
            notices.clone(),
            quick(),
        )
    }

    /// A store that refuses every write: each record is tried its attempts,
    /// then given up with an error notice naming it, so the records after it
    /// are still tried and a flush returns.
    #[test]
    fn a_store_that_always_fails_gives_each_record_up_naming_it() {
        let history = FailingHistory::failing(u32::MAX);
        let notices = Notices::shared(None);
        let recorder = recorder(&history, &notices);

        recorder.ended(&shop(), run());
        recorder.cleared(&shop(), &ProcessName::try_new("web").unwrap());
        recorder.flush();

        assert_eq!(history.attempts(), 2 * QUICK_ATTEMPTS);
        assert!(history.recorded().is_empty());
        let given_up: Vec<String> = notices
            .lock()
            .unwrap()
            .recent(&shop())
            .into_iter()
            .filter(|notice| notice.level() == &NoticeLevel::Error)
            .map(|notice| notice.message().clone())
            .collect();
        assert_eq!(given_up.len(), 2, "{given_up:?}");
        assert!(given_up[0].starts_with(GAVE_UP_NOTICE), "{given_up:?}");
        assert!(given_up[0].contains("api"), "{given_up:?}");
        assert!(given_up[1].contains("web"), "{given_up:?}");
    }

    /// A record given up does not hold back the next, which lands.
    #[test]
    fn a_record_given_up_lets_the_next_land() {
        let history = FailingHistory::failing(QUICK_ATTEMPTS);
        let notices = Notices::shared(None);
        let recorder = recorder(&history, &notices);

        recorder.cleared(&shop(), &ProcessName::try_new("web").unwrap());
        recorder.ended(&shop(), run());
        recorder.flush();

        assert_eq!(history.recorded(), [ProcessName::try_new("api").unwrap()]);
    }

    /// A write the store refuses for a while is retried until it lands,
    /// never dropped, and its failure is told once, not once per attempt.
    #[test]
    fn a_failed_write_is_retried_until_it_lands_and_told_once() {
        let history = FailingHistory::failing(3);
        let notices = Notices::shared(None);
        let recorder = recorder(&history, &notices);

        recorder.ended(&shop(), run());
        recorder.flush();

        assert_eq!(history.recorded(), [ProcessName::try_new("api").unwrap()]);
        assert_eq!(history.attempts(), 4);
        let told = notices.lock().unwrap().recent(&shop());
        assert_eq!(told.len(), 1, "{told:?}");
        assert!(told[0].message().starts_with(RUN_FAILED_NOTICE), "{told:?}");
    }
}
