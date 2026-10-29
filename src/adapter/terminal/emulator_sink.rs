use std::sync::{Mutex, MutexGuard, PoisonError};

/// Runtime state the VT event listener records for the owning emulator to drain:
/// bytes the terminal wants written back to the child (device-status and cursor
/// reports, primary/secondary DA, and so on).
#[derive(Default)]
pub(super) struct EmulatorSink {
    pub(super) pty_writes: Vec<u8>,
}

/// Locks the sink, recovering the guard if a panic poisoned it: the buffer holds
/// plain bytes, so a half-finished push is harmless.
pub(super) fn lock_sink(sink: &Mutex<EmulatorSink>) -> MutexGuard<'_, EmulatorSink> {
    sink.lock().unwrap_or_else(PoisonError::into_inner)
}
