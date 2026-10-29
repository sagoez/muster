use getset::Getters;
use serde::Serialize;
use typed_builder::TypedBuilder;

use super::condition_view::ConditionView;
use crate::domain::process::ReadyCheck;

/// A stored readiness check, in the shape `process_add` and `--ready-*`
/// take it: one of `log` (with `regex`), `port` or `http`, and a `timeout`
/// such as `60s`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct ReadyView {
    /// What it waits for.
    #[serde(flatten)]
    probe: ConditionView,
    /// How long a run may take to pass it.
    timeout: String,
}

impl From<&ReadyCheck> for ReadyView {
    fn from(check: &ReadyCheck) -> Self {
        Self {
            probe: ConditionView::from(check.probe()),
            timeout: humantime::format_duration(check.timeout().into_inner()).to_string(),
        }
    }
}
