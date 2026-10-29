use std::ops::ControlFlow;

use super::action::Action;
use crate::adapter::service::ProcessStatus;

/// Hears, each time a command looks, which of the processes it acted on are
/// still on their way, so a person can watch the wait, and may end it.
pub trait SettleWatch {
    /// Asked just before the command sends its request; breaking cancels it
    /// with nothing sent, as when the person interrupted it already.
    fn sending(&mut self) -> ControlFlow<()>;

    /// `waiting` lists the processes still on their way under `action`; it
    /// is empty once the command stops waiting. Breaking ends the wait at
    /// once, as when the person interrupted it.
    fn waiting(&mut self, action: Action, waiting: &[&ProcessStatus]) -> ControlFlow<()>;
}
