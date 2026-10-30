use std::sync::{Arc, Mutex, atomic::AtomicBool};

/// The two flags `SIGINT` drives once a guard first armed it: whether one
/// arrived, and whether no guard is armed, in which case it ends the process
/// as it would have without muster's handler; and how many guards are armed.
pub(super) struct Flags {
    /// Set by each `SIGINT`.
    pub(super) interrupted: Arc<AtomicBool>,
    /// Set while no guard is armed.
    pub(super) disarmed: Arc<AtomicBool>,
    /// Guards alive now; only the first arms and only the last disarms, so a
    /// prompt's guard inside a spinner's neither clears nor ends the
    /// spinner's.
    pub(super) armed: Mutex<usize>,
}
