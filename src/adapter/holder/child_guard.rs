use crate::domain::port::ProcessHandle;

/// The running child, force-killed when dropped before its exit was seen, so
/// a holder that stops serving on any path never leaves it running unreachable.
pub(super) struct ChildGuard {
    /// The child's handle.
    pub(super) handle: Box<dyn ProcessHandle>,
    /// Whether the child's exit has been observed.
    pub(super) exited: bool,
}

impl Drop for ChildGuard {
    /// Kills a child whose exit was never observed.
    fn drop(&mut self) {
        if !self.exited {
            let _ = self.handle.kill();
        }
    }
}
