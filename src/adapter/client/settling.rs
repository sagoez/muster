use std::time::{Duration, SystemTime};

use getset::CopyGetters;
use typed_builder::TypedBuilder;

use super::action::Action;
use crate::{adapter::service::ProjectStatus, domain::value::ProcessName};

/// A wait with no bound beyond the action's own timeout.
pub const UNBOUNDED: Duration = Duration::MAX;

/// What a command waits for once it asked the service to act: the
/// processes `names` it acted on with `action`, listed as `before` when the
/// request was `sent`, waited for at most `bound`.
#[derive(Debug, Clone, Copy, CopyGetters, TypedBuilder)]
#[getset(get_copy = "pub(super)")]
pub(super) struct Settling<'a> {
    /// What was asked.
    action: Action,
    /// The project as it was listed before the request.
    before: &'a ProjectStatus,
    /// The processes it acted on.
    names: &'a [ProcessName],
    /// When the request was sent; notices from then on belong to the report.
    sent: SystemTime,
    /// The caller's bound on the wait.
    #[builder(default = UNBOUNDED)]
    bound: Duration,
}
