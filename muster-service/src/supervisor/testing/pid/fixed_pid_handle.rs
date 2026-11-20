use muster_domain::value::Pid;

use crate::supervisor::testing::StubHandle;

/// A handle that reports a fixed pid and accepts every request.
pub(crate) struct FixedPidHandle(Pid);

impl FixedPidHandle {
    /// A handle reporting `pid`.
    pub(crate) fn new(pid: Pid) -> Self {
        Self(pid)
    }
}

impl StubHandle for FixedPidHandle {
    fn process_id(&self) -> Option<Pid> {
        Some(self.0)
    }
}
