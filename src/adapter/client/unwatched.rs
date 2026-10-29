use std::ops::ControlFlow;

use super::{action::Action, settle_watch::SettleWatch};
use crate::adapter::service::ProcessStatus;

/// A watch nobody looks at, for callers with no one to show the wait to.
pub struct Unwatched;

impl SettleWatch for Unwatched {
    fn sending(&mut self) -> ControlFlow<()> {
        ControlFlow::Continue(())
    }

    fn waiting(&mut self, _action: Action, _waiting: &[&ProcessStatus]) -> ControlFlow<()> {
        ControlFlow::Continue(())
    }
}
