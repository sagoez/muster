use getset::{CopyGetters, Getters};
use serde::Serialize;
use typed_builder::TypedBuilder;

use crate::adapter::{
    client::ExitView,
    service::{ExitReason, LastExit},
};

/// How a process's last run ended, as `stack_status` shows it: its status or
/// signal, why it ended, and when, as RFC 3339 UTC to the second.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Getters, CopyGetters, TypedBuilder)]
pub struct LastExitView {
    /// Its exit status or signal.
    #[serde(flatten)]
    #[getset(get = "pub")]
    exit: ExitView,
    /// Why it ended.
    #[getset(get_copy = "pub")]
    reason: ExitReason,
    /// When it ended, when the service saw it end.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[getset(get = "pub")]
    at: Option<String>,
}

impl From<&LastExit> for LastExitView {
    fn from(exit: &LastExit) -> Self {
        Self {
            exit: ExitView::from(exit.outcome()),
            reason: exit.reason(),
            at: exit
                .at()
                .map(|at| humantime::format_rfc3339_seconds(at).to_string()),
        }
    }
}
