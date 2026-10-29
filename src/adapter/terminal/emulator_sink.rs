use std::{
    mem,
    sync::{Mutex, MutexGuard, PoisonError},
};

/// Runtime state the VT event listener records for the owning emulator to drain:
/// bytes the terminal wants written back to the child (device-status and cursor
/// reports, primary/secondary DA, and so on).
#[derive(Default)]
pub(super) struct EmulatorSink {
    /// Bytes owed to the child, in the order the terminal produced them.
    pty_writes: Vec<u8>,
}

impl EmulatorSink {
    /// Queues `bytes` to be written back to the child.
    pub(super) fn write(&mut self, bytes: &[u8]) {
        self.pty_writes.extend_from_slice(bytes);
    }

    /// The bytes owed to the child, leaving none behind.
    pub(super) fn take(&mut self) -> Vec<u8> {
        mem::take(&mut self.pty_writes)
    }
}

/// Locks the sink, recovering the guard if a panic poisoned it: the buffer holds
/// plain bytes, so a half-finished push is harmless.
pub(super) fn lock_sink(sink: &Mutex<EmulatorSink>) -> MutexGuard<'_, EmulatorSink> {
    sink.lock().unwrap_or_else(PoisonError::into_inner)
}
