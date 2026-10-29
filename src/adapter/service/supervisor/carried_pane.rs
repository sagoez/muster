use std::time::Instant;

use super::config_membership::ConfigMembership;
use crate::domain::{
    process::{ExitIntent, ProcessState},
    value::Pid,
};

/// What a pane keeps across a reconnect to its holder: everything about it
/// that is not tied to one connection.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CarriedPane {
    /// What the pane's next exit means, such as a stop in flight.
    pub(super) intent: ExitIntent,
    /// Whether the pane still belongs to the current config.
    pub(super) membership: ConfigMembership,
    /// When the process really started.
    pub(super) started_at: Instant,
    /// The state before the stream broke.
    pub(super) state: Option<ProcessState>,
    /// The child's pid on the broken connection.
    pub(super) pid: Option<Pid>,
}
