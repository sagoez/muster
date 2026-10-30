use std::{
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::{Instant, SystemTime},
};

use crossbeam_channel::{Receiver, Sender, bounded, unbounded};

use super::{
    notice::Notice, notice_level::NoticeLevel, record_retry::RecordRetry,
    recorder_stores::RecorderStores, shared_notices::SharedNotices, store_record::StoreRecord,
};
use crate::domain::{
    config::ConfigError,
    occupancy::PendingIdleStop,
    process::EndedRun,
    value::{EditId, ProcessName, ProjectRoot},
};

/// Prefix of the notice for a run end the store was too busy to take yet;
/// the store's error follows.
const RUN_BUSY_NOTICE: &str = "could not record how the process last ended yet, retrying";
/// Prefix of the notice for a raised edit mark the store was too busy to take
/// yet; the store's error follows.
const RAISED_BUSY_NOTICE: &str = "could not record which agent edits were raised yet, retrying";
/// Prefix of the notice for a record the store stayed busy for past the
/// retry budget; what it was, the budget and the store's last error follow.
const GAVE_UP_NOTICE: &str = "gave up recording";
/// Prefix of the notice for a record the store refused for a reason a retry
/// cannot change; what it was and the store's error follow.
const REFUSED_NOTICE: &str = "could not record";
/// Prefix of the notice for a record still waiting when a flush's drain
/// ended; what it was follows.
const STOPPED_NOTICE: &str = "the service stopped before recording";
/// Prefix of the notice for records a flush gave up waiting on; their count
/// follows.
const UNCONFIRMED_NOTICE: &str = "the service stopped before these records were confirmed stored:";
/// What a run end was, before the process.
const ENDED_RECORD: &str = "how a run ended for";
/// What a cleared run end was, before the process.
const CLEARED_RECORD: &str = "that a run ended on request for";
/// What a raised edit mark was, before the newest edit.
const RAISED_RECORD: &str = "agent edits raised through";
/// Prefix of the notice for a pending idle stop the store was too busy to
/// take yet; the store's error follows.
const IDLE_BUSY_NOTICE: &str = "could not record the project's pending idle stop yet, retrying";
/// What an armed idle stop was.
const IDLE_ARMED_RECORD: &str = "the project's pending idle stop";
/// What a cleared idle stop was.
const IDLE_CLEARED_RECORD: &str = "that the project's idle stop is no longer pending";

/// Stores what the service keeps across restarts, how each process's runs
/// end, which agent edits it raised and which idle stops are pending, on a
/// thread of its own, so a slow or
/// locked database never stalls the loop. Writes land in the order they are
/// sent. One the store is only too busy for is retried under a
/// [`RecordRetry`], its first refusal a warning notice, and given up with an
/// error notice naming it once the retry budget runs out; one refused for any
/// other reason is given up at once with an error notice. So a database that
/// stays locked or broken never holds back the records after it. Clones
/// share the thread, which ends once every clone is dropped.
#[derive(Clone)]
pub struct StoreRecorder {
    /// Where writes go to the thread.
    sender: Sender<StoreRecord>,
    /// When the records still waiting stop being tried, while a flush drains.
    drain_by: Arc<Mutex<Option<Instant>>>,
    /// Records sent and not yet landed or given up.
    pending: Arc<AtomicUsize>,
    /// Where a flush reports records it gave up waiting on.
    notices: SharedNotices,
    /// How records are retried and how long a flush drains.
    retry: RecordRetry,
}

impl StoreRecorder {
    /// Starts the thread writing each record to its store in `stores` under
    /// `retry`, telling `notices` of a write that has to wait or was given
    /// up.
    pub fn spawn(stores: RecorderStores, notices: SharedNotices, retry: RecordRetry) -> Self {
        let (sender, records) = unbounded();
        let drain_by = Arc::new(Mutex::new(None));
        let pending = Arc::new(AtomicUsize::new(0));
        let worker = (
            Arc::clone(&drain_by),
            Arc::clone(&pending),
            Arc::clone(&notices),
        );
        thread::spawn(move || {
            let (drain_by, pending, notices) = worker;
            write_all(&records, &stores, &notices, retry, &drain_by, &pending);
        });
        Self {
            sender,
            drain_by,
            pending,
            notices,
            retry,
        }
    }

    /// Records that `run` of `project` ended on its own.
    pub fn ended(&self, project: &ProjectRoot, run: EndedRun) {
        self.send(StoreRecord::Ended {
            project: project.clone(),
            run,
        });
    }

    /// Forgets how `process` of `project` last ended.
    pub fn cleared(&self, project: &ProjectRoot, process: &ProcessName) {
        self.send(StoreRecord::Cleared {
            project: project.clone(),
            process: process.clone(),
        });
    }

    /// Records that every agent edit of `project` up to `through` was raised.
    pub fn edits_raised(&self, project: &ProjectRoot, through: EditId) {
        self.send(StoreRecord::EditsRaised {
            project: project.clone(),
            through,
        });
    }

    /// Records `stop`, a project's idle stop now pending.
    pub fn idle_armed(&self, stop: PendingIdleStop) {
        self.send(StoreRecord::IdleArmed(stop));
    }

    /// Records that `project`'s idle stop is no longer pending.
    pub fn idle_cleared(&self, project: &ProjectRoot) {
        self.send(StoreRecord::IdleCleared(project.clone()));
    }

    /// Drains every write sent before it, so a service exiting leaves its
    /// last records stored. Records keep trying for the retry's `drain` only:
    /// past it each one still waiting is given up with a notice naming it.
    /// The wait ends `settle` past the drain at the latest, and a notice then
    /// counts the records never confirmed.
    pub fn flush(&self) {
        let drain_by = Instant::now() + self.retry.drain();
        *self.drain_by.lock().unwrap_or_else(PoisonError::into_inner) = Some(drain_by);
        let (done, landed) = bounded(1);
        let answered = self
            .sender
            .send(StoreRecord::Flush { done, drain_by })
            .is_ok()
            && landed
                .recv_timeout(self.retry.drain() + self.retry.settle())
                .is_ok();
        let unconfirmed = self.pending.load(Ordering::SeqCst);
        if !answered && unconfirmed > 0 {
            report_unconfirmed(&self.notices, unconfirmed);
        }
    }

    /// Hands `record` to the thread, counting it until it is done.
    fn send(&self, record: StoreRecord) {
        self.pending.fetch_add(1, Ordering::SeqCst);
        if self.sender.send(record).is_err() {
            self.pending.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

/// Applies each record from `records` in order until every sender is gone,
/// landing each as [`land`] does and counting it off `pending`, and ends a
/// flush's drain once the flush is answered.
fn write_all(
    records: &Receiver<StoreRecord>,
    stores: &RecorderStores,
    notices: &SharedNotices,
    retry: RecordRetry,
    drain_by: &Mutex<Option<Instant>>,
    pending: &AtomicUsize,
) {
    for record in records {
        if let StoreRecord::Flush {
            done,
            drain_by: set,
        } = record
        {
            let mut slot = drain_by.lock().unwrap_or_else(PoisonError::into_inner);
            if *slot == Some(set) {
                *slot = None;
            }
            drop(slot);
            let _ = done.send(());
            continue;
        }
        land(&record, stores, notices, retry, drain_by);
        pending.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Writes `record`, retrying it while the store is only busy until `retry`'s
/// budget or a flush's drain runs out. Tells `notices` the first time it is
/// busy, and when it is given up.
fn land(
    record: &StoreRecord,
    stores: &RecorderStores,
    notices: &SharedNotices,
    retry: RecordRetry,
    drain_by: &Mutex<Option<Instant>>,
) {
    let started = Instant::now();
    let mut pause = retry.first();
    let mut told = false;
    loop {
        if drain_passed(drain_by, Instant::now()) {
            report(notices, record, |what| format!("{STOPPED_NOTICE} {what}"));
            return;
        }
        let Err(error) = apply(record, stores) else {
            return;
        };
        if !error.is_busy() {
            report(notices, record, |what| {
                format!("{REFUSED_NOTICE} {what}: {error}")
            });
            return;
        }
        let resume = Instant::now() + pause;
        if resume.duration_since(started) > retry.budget() {
            report(notices, record, |what| {
                format!(
                    "{GAVE_UP_NOTICE} {what} after {}: {error}",
                    humantime::format_duration(retry.budget())
                )
            });
            return;
        }
        if drain_passed(drain_by, resume) {
            report(notices, record, |what| format!("{STOPPED_NOTICE} {what}"));
            return;
        }
        if !told {
            tell(notices, record, &error);
            told = true;
        }
        thread::sleep(pause);
        pause = pause.saturating_mul(2).min(retry.max());
    }
}

/// Whether a flush's drain ends by `at`.
fn drain_passed(drain_by: &Mutex<Option<Instant>>, at: Instant) -> bool {
    drain_by
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .is_some_and(|deadline| at >= deadline)
}

/// Writes `record` to the store it belongs to.
///
/// # Errors
/// Returns the store's error when the write fails.
fn apply(record: &StoreRecord, stores: &RecorderStores) -> Result<(), ConfigError> {
    match record {
        StoreRecord::Ended { project, run } => stores.runs.record_end(project, run),
        StoreRecord::Cleared { project, process } => stores.runs.clear_end(project, process),
        StoreRecord::EditsRaised { project, through } => {
            stores.raised.mark_raised(project, *through)
        },
        StoreRecord::IdleArmed(stop) => stores.idle_stops.arm_idle_stop(stop),
        StoreRecord::IdleCleared(project) => stores.idle_stops.clear_idle_stop(project),
        StoreRecord::Flush { .. } => Ok(()),
    }
}

/// Tells `notices` that the store was too busy for `record`, failing with
/// `error`, and it will be retried.
fn tell(notices: &SharedNotices, record: &StoreRecord, error: &ConfigError) {
    let busy = match record {
        StoreRecord::Ended { .. } | StoreRecord::Cleared { .. } => RUN_BUSY_NOTICE,
        StoreRecord::EditsRaised { .. } => RAISED_BUSY_NOTICE,
        StoreRecord::IdleArmed(_) | StoreRecord::IdleCleared(_) => IDLE_BUSY_NOTICE,
        StoreRecord::Flush { .. } => return,
    };
    push(
        notices,
        record,
        NoticeLevel::Warning,
        format!("{busy}: {error}"),
    );
}

/// Tells `notices`, as an error, that `record` was given up, in the message
/// `say` makes of what it was.
fn report(notices: &SharedNotices, record: &StoreRecord, say: impl FnOnce(&str) -> String) {
    let what = match record {
        StoreRecord::Ended { run, .. } => format!("{ENDED_RECORD} {}", run.process()),
        StoreRecord::Cleared { process, .. } => format!("{CLEARED_RECORD} {process}"),
        StoreRecord::EditsRaised { through, .. } => format!("{RAISED_RECORD} {through}"),
        StoreRecord::IdleArmed(_) => IDLE_ARMED_RECORD.to_string(),
        StoreRecord::IdleCleared(_) => IDLE_CLEARED_RECORD.to_string(),
        StoreRecord::Flush { .. } => return,
    };
    push(notices, record, NoticeLevel::Error, say(&what));
}

/// Pushes `message` at `level` onto `notices`, about the project and process
/// `record` is of.
fn push(notices: &SharedNotices, record: &StoreRecord, level: NoticeLevel, message: String) {
    let (project, process) = match record {
        StoreRecord::Ended { project, run } => (project, Some(run.process().clone())),
        StoreRecord::Cleared { project, process } => (project, Some(process.clone())),
        StoreRecord::EditsRaised { project, .. } | StoreRecord::IdleCleared(project) => {
            (project, None)
        },
        StoreRecord::IdleArmed(stop) => (stop.project(), None),
        StoreRecord::Flush { .. } => return,
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

/// Tells `notices`, as an error about the whole service, that `count`
/// records were never confirmed stored before a flush stopped waiting.
fn report_unconfirmed(notices: &SharedNotices, count: usize) {
    notices.lock().unwrap_or_else(PoisonError::into_inner).push(
        Notice::builder()
            .at(SystemTime::now())
            .project(None)
            .process(None)
            .level(NoticeLevel::Error)
            .message(format!("{UNCONFIRMED_NOTICE} {count}"))
            .build(),
    );
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::{
        adapter::{
            service::{failing_history::FailingHistory, notices::Notices},
            test_wait::wait_until,
        },
        domain::{pty::ExitOutcome, value::ProcessName},
    };

    /// Longest a test lets a flush take before calling it unbounded.
    const FLUSH_BOUND: Duration = Duration::from_secs(5);

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

    /// A retry that backs off in milliseconds within `budget`, draining a
    /// flush for `drain`.
    fn quick(budget: Duration, drain: Duration) -> RecordRetry {
        RecordRetry::builder()
            .first(Duration::from_millis(1))
            .max(Duration::from_millis(4))
            .budget(budget)
            .drain(drain)
            .settle(Duration::from_millis(50))
            .build()
    }

    /// A recorder over `history` telling `notices`, retrying as `retry`.
    fn recorder(
        history: &FailingHistory,
        notices: &SharedNotices,
        retry: RecordRetry,
    ) -> StoreRecorder {
        StoreRecorder::spawn(
            RecorderStores::builder()
                .runs(Box::new(history.clone()))
                .raised(Box::new(history.clone()))
                .idle_stops(Box::new(history.clone()))
                .build(),
            notices.clone(),
            retry,
        )
    }

    /// A retry with room to spare for every test that expects records to
    /// land.
    fn roomy() -> RecordRetry {
        quick(FLUSH_BOUND, FLUSH_BOUND)
    }

    /// The messages of every notice at `level` about the shop.
    fn messages(notices: &SharedNotices, level: NoticeLevel) -> Vec<String> {
        notices
            .lock()
            .unwrap()
            .recent(&shop())
            .into_iter()
            .filter(|notice| notice.level() == &level)
            .map(|notice| notice.message().clone())
            .collect()
    }

    /// A store that refuses a write for a reason a retry cannot change gives
    /// each record up at once with an error notice naming it, never retried
    /// and never warned about, so the records after it are still tried.
    #[test]
    fn a_deterministic_refusal_gives_the_record_up_at_once_naming_it() {
        let history = FailingHistory::broken(u32::MAX);
        let notices = Notices::shared(None);
        let recorder = recorder(&history, &notices, roomy());

        recorder.ended(&shop(), run());
        recorder.cleared(&shop(), &ProcessName::try_new("web").unwrap());
        recorder.flush();

        assert_eq!(history.attempts(), 2, "no retries");
        assert!(messages(&notices, NoticeLevel::Warning).is_empty());
        let refused = messages(&notices, NoticeLevel::Error);
        assert_eq!(refused.len(), 2, "{refused:?}");
        assert!(refused[0].starts_with(REFUSED_NOTICE), "{refused:?}");
        assert!(refused[0].contains("api"), "{refused:?}");
        assert!(refused[1].contains("web"), "{refused:?}");
    }

    /// A store locked for longer than the retry budget retries each record
    /// only within it, then gives it up with an error notice naming it.
    #[test]
    fn a_long_lock_is_retried_within_the_budget_then_given_up() {
        let history = FailingHistory::failing(u32::MAX);
        let notices = Notices::shared(None);
        let budget = Duration::from_millis(30);
        let recorder = recorder(&history, &notices, quick(budget, FLUSH_BOUND));
        let started = Instant::now();

        recorder.ended(&shop(), run());
        recorder.cleared(&shop(), &ProcessName::try_new("web").unwrap());
        recorder.flush();

        assert!(started.elapsed() < FLUSH_BOUND, "{:?}", started.elapsed());
        assert!(history.attempts() > 2, "a busy store is retried");
        assert!(history.recorded().is_empty());
        let given_up = messages(&notices, NoticeLevel::Error);
        assert_eq!(given_up.len(), 2, "{given_up:?}");
        assert!(given_up[0].starts_with(GAVE_UP_NOTICE), "{given_up:?}");
        assert!(given_up[0].contains("api"), "{given_up:?}");
        assert!(given_up[1].contains("web"), "{given_up:?}");
    }

    /// A flush on a store that stays locked drains within its bound however
    /// long the retry budget, and names each record it could not land.
    #[test]
    fn a_flush_drains_within_its_bound_naming_what_did_not_land() {
        let history = FailingHistory::failing(u32::MAX);
        let notices = Notices::shared(None);
        let recorder = recorder(
            &history,
            &notices,
            RecordRetry::builder()
                .first(Duration::from_millis(1))
                .max(Duration::from_millis(4))
                .budget(Duration::from_secs(3600))
                .drain(Duration::from_millis(30))
                .settle(FLUSH_BOUND)
                .build(),
        );
        let started = Instant::now();

        recorder.ended(&shop(), run());
        recorder.cleared(&shop(), &ProcessName::try_new("web").unwrap());
        wait_until("the first record to be retried", || history.attempts() > 1);
        recorder.flush();

        assert!(started.elapsed() < FLUSH_BOUND, "{:?}", started.elapsed());
        let stopped = messages(&notices, NoticeLevel::Error);
        assert_eq!(stopped.len(), 2, "{stopped:?}");
        assert!(stopped[0].starts_with(STOPPED_NOTICE), "{stopped:?}");
        assert!(stopped[0].contains("api"), "{stopped:?}");
        assert!(stopped[1].contains("web"), "{stopped:?}");
    }

    /// A flush that gives up waiting on a write stuck in the store reports
    /// how many records it could not confirm.
    #[test]
    fn a_flush_past_its_settle_counts_the_records_it_could_not_confirm() {
        let (entered, inside) = unbounded();
        let (release, held) = bounded::<()>(0);
        let history = FailingHistory::held(entered, held);
        let notices = Notices::shared(None);
        let recorder = recorder(
            &history,
            &notices,
            quick(FLUSH_BOUND, Duration::from_millis(10)),
        );

        recorder.ended(&shop(), run());
        inside
            .recv_timeout(FLUSH_BOUND)
            .expect("the write reaches the store");
        recorder.flush();
        drop(release);

        let told = notices.lock().unwrap().recent(&shop());
        assert_eq!(told.len(), 1, "{told:?}");
        assert!(told[0].project().is_none(), "{told:?}");
        assert_eq!(told[0].message(), &format!("{UNCONFIRMED_NOTICE} 1"));
    }

    /// A record given up does not hold back the next, which lands.
    #[test]
    fn a_record_given_up_lets_the_next_land() {
        let history = FailingHistory::broken(1);
        let notices = Notices::shared(None);
        let recorder = recorder(&history, &notices, roomy());

        recorder.cleared(&shop(), &ProcessName::try_new("web").unwrap());
        recorder.ended(&shop(), run());
        recorder.flush();

        assert_eq!(history.recorded(), [ProcessName::try_new("api").unwrap()]);
    }

    /// A write the store is busy for a while is retried until it lands,
    /// never dropped, and its failure is told once, not once per attempt.
    #[test]
    fn a_busy_write_is_retried_until_it_lands_and_told_once() {
        let history = FailingHistory::failing(3);
        let notices = Notices::shared(None);
        let recorder = recorder(&history, &notices, roomy());

        recorder.ended(&shop(), run());
        recorder.flush();

        assert_eq!(history.recorded(), [ProcessName::try_new("api").unwrap()]);
        assert_eq!(history.attempts(), 4);
        let told = notices.lock().unwrap().recent(&shop());
        assert_eq!(told.len(), 1, "{told:?}");
        assert!(told[0].message().starts_with(RUN_BUSY_NOTICE), "{told:?}");
    }
}
