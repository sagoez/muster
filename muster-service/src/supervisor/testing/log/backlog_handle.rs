use muster_domain::value::OutputCursor;

use crate::supervisor::testing::{
    StubHandle,
    fixtures::{BACKLOG, BACKLOG_RUN_START},
};

/// A handle whose run began at [`BACKLOG_RUN_START`] and logged [`BACKLOG`]
/// before it connected, as a holder reached after its child printed.
pub(crate) struct BacklogHandle;

impl StubHandle for BacklogHandle {
    fn run_start(&self) -> Option<OutputCursor> {
        Some(OutputCursor::new(BACKLOG_RUN_START))
    }

    fn take_backlog(&mut self) -> Option<(OutputCursor, Vec<u8>)> {
        Some((OutputCursor::new(BACKLOG_RUN_START), BACKLOG.to_vec()))
    }
}
