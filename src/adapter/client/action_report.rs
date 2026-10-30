use getset::{CopyGetters, Getters};
use serde::Serialize;
use typed_builder::TypedBuilder;

use super::{
    action::Action, notice_view::NoticeView, process_outcome::ProcessOutcome,
    project_identity_view::ProjectIdentityView, wait_end::WaitEnd,
};
use crate::{adapter::service::NoticeLevel, domain::value::ProcessName};

/// What a command that acts on processes did, once they settled: the
/// `--json` output of `up`, `down`, `start` and their siblings.
#[derive(Debug, Clone, PartialEq, Serialize, Getters, CopyGetters, TypedBuilder)]
pub struct ActionReport {
    /// The project.
    #[getset(get = "pub")]
    project: ProjectIdentityView,
    /// What was asked.
    #[getset(get_copy = "pub")]
    action: Action,
    /// Each process it acted on.
    #[getset(get = "pub")]
    processes: Vec<ProcessOutcome>,
    /// What supervision reported about those processes while it ran, such as
    /// why a start failed.
    #[getset(get = "pub")]
    notices: Vec<NoticeView>,
    /// Other notices raised meanwhile, about processes it did not act on;
    /// context only, never a reason to fail.
    #[getset(get = "pub")]
    other_notices: Vec<NoticeView>,
    /// How the wait for its processes ended.
    #[builder(default = WaitEnd::Settled)]
    #[getset(get_copy = "pub")]
    wait_ended: WaitEnd,
    /// Whether the project has no process `up` starts, as the person reads
    /// an `up` that started nothing.
    #[serde(skip)]
    #[builder(default)]
    #[getset(get_copy = "pub")]
    none_autostart: bool,
}

impl ActionReport {
    /// The report saying whether the project has no process `up` starts.
    pub fn with_none_autostart(mut self, none_autostart: bool) -> Self {
        self.none_autostart = none_autostart;
        self
    }

    /// The report with `more` processes listed after its own.
    pub fn with_more(mut self, more: impl IntoIterator<Item = ProcessOutcome>) -> Self {
        self.processes.extend(more);
        self
    }

    /// The report with the notices about `names`, which it also lists, moved
    /// from the context into its own notices.
    pub fn claiming_notices_about(mut self, names: &[ProcessName]) -> Self {
        let (claimed, others): (Vec<NoticeView>, Vec<NoticeView>) =
            self.other_notices.into_iter().partition(|notice| {
                notice
                    .process()
                    .as_ref()
                    .is_some_and(|process| names.contains(process))
            });
        self.notices.extend(claimed);
        self.other_notices = others;
        self
    }

    /// Whether a process it acted on did not end up as asked, or
    /// supervision reported an error about one, so the command fails. A
    /// process still on its way when the caller's bound cut the wait short
    /// has not failed.
    pub fn failed(&self) -> bool {
        let cut_short = self.wait_ended == WaitEnd::CutShort;
        self.processes
            .iter()
            .map(|process| process.outcome())
            .any(|outcome| outcome.is_failure() && !(cut_short && outcome.is_unsettled()))
            || self
                .notices
                .iter()
                .any(|notice| *notice.level() == NoticeLevel::Error)
    }
}
