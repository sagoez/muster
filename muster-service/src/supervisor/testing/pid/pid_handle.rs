use muster_domain::value::Pid;

use crate::supervisor::testing::{StubHandle, fixtures::fake_launcher_pid};

/// A handle that reports a pid, like the real PTY handle does.
pub(crate) struct PidHandle;

impl StubHandle for PidHandle {
    fn process_id(&self) -> Option<Pid> {
        Some(fake_launcher_pid())
    }
}
