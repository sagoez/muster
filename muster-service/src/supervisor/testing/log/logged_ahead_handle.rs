use muster_domain::value::OutputCursor;

use crate::supervisor::testing::{StubHandle, fixtures::LOGGED_AHEAD_END};

/// A handle whose log already ends at [`LOGGED_AHEAD_END`], ahead of any
/// output it delivered, as a holder's log is while its output is in flight.
pub(crate) struct LoggedAheadHandle;

impl StubHandle for LoggedAheadHandle {
    fn output_end(&self) -> Option<OutputCursor> {
        Some(OutputCursor::new(LOGGED_AHEAD_END))
    }
}
