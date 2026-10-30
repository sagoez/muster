use getset::{CopyGetters, Getters};
use serde::Serialize;
use typed_builder::TypedBuilder;

use super::condition_view::ConditionView;
use crate::domain::process::ReadyCheck;

/// A stored readiness check, as `--json` and MCP show it: one of `log` (with
/// `regex`), `port` or `http`, and a `timeout` in seconds.
#[derive(Debug, Clone, PartialEq, Serialize, Getters, CopyGetters, TypedBuilder)]
pub struct ReadyView {
    /// What it waits for.
    #[serde(flatten)]
    #[getset(get = "pub")]
    probe: ConditionView,
    /// Seconds a run may take to pass it.
    #[getset(get_copy = "pub")]
    timeout: f64,
}

impl From<&ReadyCheck> for ReadyView {
    fn from(check: &ReadyCheck) -> Self {
        Self {
            probe: ConditionView::from(check.probe()),
            timeout: check.timeout().into_inner().as_secs_f64(),
        }
    }
}
