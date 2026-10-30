use std::sync::{Arc, Mutex, atomic::AtomicBool};

/// What `SIGINT` drives once a guard first armed it: whether one arrived,
/// and how many guards are armed.
pub(super) struct Flags {
    /// Set by each `SIGINT`.
    pub(super) interrupted: Arc<AtomicBool>,
    /// Guards alive now; only the first clears the flag, so a prompt's guard
    /// inside a spinner's never clears the spinner's pending interrupt.
    pub(super) armed: Mutex<usize>,
}
