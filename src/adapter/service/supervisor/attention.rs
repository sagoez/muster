use std::{cell::RefCell, collections::HashSet, sync::PoisonError, time::SystemTime};

use crate::{
    adapter::service::{
        EventDraft, EventKind, Notice, NoticeLevel, PortBlock, Refusal, SharedEventLog,
        SharedNotices,
    },
    domain::{
        notification::Notification,
        port::Notifier,
        value::{PaneId, ProcessName, ProjectRoot},
    },
};

/// Where supervision tells the user what they should know: notices, the event
/// log and the desktop, with the streaks and batches that decide what is told.
pub(crate) struct Attention {
    /// Where supervision reports what the user should know.
    notices: SharedNotices,
    /// Delivers desktop notifications.
    notifier: Option<Box<dyn Notifier>>,
    /// Where lifecycle events are recorded.
    event_log: SharedEventLog,
    /// While a project-wide request runs, each error raised about one of its
    /// processes, which is why that process was left alone.
    batch_refusals: RefCell<Option<Vec<Refusal>>>,
    /// Panes whose last holder screen read failed, so a failure streak is
    /// reported once.
    failing_screen_reads: HashSet<PaneId>,
}

impl Attention {
    /// Attention told through `notices`, `event_log` and, when given,
    /// `notifier`.
    pub(super) fn new(
        notices: SharedNotices,
        event_log: SharedEventLog,
        notifier: Option<Box<dyn Notifier>>,
    ) -> Self {
        Self {
            notices,
            notifier,
            event_log,
            batch_refusals: RefCell::new(None),
            failing_screen_reads: HashSet::new(),
        }
    }

    /// Swaps in desktop notification delivery, for a test to inject a fake.
    #[cfg(test)]
    pub(super) fn set_notifier(&mut self, notifier: Box<dyn Notifier>) {
        self.notifier = Some(notifier);
    }

    /// Records a notice about `process` of `project`, or the project as a
    /// whole: in the notices and the event log, and as a refusal of the
    /// project-wide request running, when it is an error about a process.
    pub(super) fn notify(
        &self,
        project: &ProjectRoot,
        level: NoticeLevel,
        process: Option<ProcessName>,
        message: String,
    ) {
        self.raise(project, level, process, message, None);
    }

    /// Records that `name` of `project` was not started because of `block`,
    /// as an error [`Self::notify`] records, its refusal carrying the block.
    pub(super) fn notify_blocked(
        &self,
        project: &ProjectRoot,
        name: ProcessName,
        block: PortBlock,
    ) {
        let message = block.to_string();
        self.raise(
            project,
            NoticeLevel::Error,
            Some(name),
            message,
            Some(block),
        );
    }

    /// Records the notice [`Self::notify`] describes, a refusal of the
    /// running batch carrying `block`.
    fn raise(
        &self,
        project: &ProjectRoot,
        level: NoticeLevel,
        process: Option<ProcessName>,
        message: String,
        block: Option<PortBlock>,
    ) {
        if level == NoticeLevel::Error
            && let (Some(name), Some(refusals)) =
                (process.as_ref(), self.batch_refusals.borrow_mut().as_mut())
        {
            refusals.push(
                Refusal::builder()
                    .name(name.clone())
                    .reason(message.clone())
                    .block(block)
                    .build(),
            );
        }
        self.record(
            EventDraft::builder()
                .project(Some(project.clone()))
                .process(process.clone())
                .kind(EventKind::Notice)
                .detail(message.clone())
                .build(),
        );
        let notice = Notice::builder()
            .at(SystemTime::now())
            .project(Some(project.clone()))
            .process(process)
            .level(level)
            .message(message)
            .build();
        self.notices
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(notice);
    }

    /// Records `event` in the event log.
    pub(super) fn record(&self, event: EventDraft) {
        self.event_log
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .record(event);
    }

    /// Raises `notification` on the desktop, when a notifier is wired.
    pub(super) fn alert_desktop(&self, notification: &Notification) {
        if let Some(notifier) = &self.notifier {
            notifier.notify(notification);
        }
    }

    /// Starts collecting the errors a project-wide request raises about its
    /// processes.
    pub(super) fn begin_batch(&self) {
        self.batch_refusals.replace(Some(Vec::new()));
    }

    /// The processes the running project-wide request refused so far.
    pub(super) fn refused_so_far(&self) -> Vec<ProcessName> {
        self.batch_refusals
            .borrow()
            .iter()
            .flatten()
            .map(|refusal| refusal.name().clone())
            .collect()
    }

    /// Ends the running project-wide request, returning what it refused.
    pub(super) fn end_batch(&self) -> Vec<Refusal> {
        self.batch_refusals.take().unwrap_or_default()
    }

    /// Counts a failed holder screen read of `pane`; returns whether it begins
    /// a streak, and so is told.
    pub(super) fn screen_read_failed(&mut self, pane: PaneId) -> bool {
        self.failing_screen_reads.insert(pane)
    }

    /// Ends `pane`'s streak of failed holder screen reads, for a read that
    /// worked or a pane retired from the workspace.
    pub(super) fn end_screen_read_streak(&mut self, pane: PaneId) {
        self.failing_screen_reads.remove(&pane);
    }

    /// Whether any pane is in a streak of failed holder screen reads.
    #[cfg(test)]
    pub(super) fn has_failing_screen_reads(&self) -> bool {
        !self.failing_screen_reads.is_empty()
    }
}
