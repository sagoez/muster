use std::cell::Cell;

thread_local! {
    /// Whether the next piece this thread renders panics.
    static ARMED: Cell<bool> = const { Cell::new(false) };
}

/// Makes the next piece rendered on this thread panic, for a test to prove a
/// renderer panic is contained wherever it happens.
pub fn arm() {
    ARMED.with(|armed| armed.set(true));
}

/// Panics once after [`arm`], as a renderer bug would.
pub fn fire_if_armed() {
    if ARMED.with(|armed| armed.replace(false)) {
        panic!("render fault injected by a test");
    }
}
