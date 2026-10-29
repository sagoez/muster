use std::time::Instant;

use getset::CopyGetters;
use typed_builder::TypedBuilder;

use super::config_membership::ConfigMembership;
use crate::domain::{
    process::{ExitIntent, ProcessState},
    value::Pid,
};

/// What a pane keeps across a reconnect to its holder: everything about it
/// that is not tied to one connection.
#[derive(Debug, Clone, Copy, CopyGetters, TypedBuilder)]
pub(crate) struct CarriedPane {
    /// What the pane's next exit means, such as a stop in flight.
    #[getset(get_copy = "pub(super)")]
    intent: ExitIntent,
    /// Whether the pane still belongs to the current config.
    #[getset(get_copy = "pub(super)")]
    membership: ConfigMembership,
    /// When the process really started.
    #[getset(get_copy = "pub(super)")]
    started_at: Instant,
    /// The state before the stream broke.
    #[getset(get_copy = "pub(super)")]
    state: Option<ProcessState>,
    /// The child's pid on the broken connection.
    #[getset(get_copy = "pub(super)")]
    pid: Option<Pid>,
}

impl CarriedPane {
    /// Takes `membership`, the one a reconcile gave the pane while it
    /// reconnected, when there is one.
    pub(super) fn with_membership(mut self, membership: Option<ConfigMembership>) -> Self {
        if let Some(membership) = membership {
            self.membership = membership;
        }
        self
    }
}
