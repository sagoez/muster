use std::process::Child;

/// Kills and collects a spawned child even when the test panics.
pub(super) struct Reaped(Child);

impl Reaped {
    /// A guard reaping `child`.
    pub(super) fn new(child: Child) -> Self {
        Self(child)
    }

    /// The child's process id.
    pub(super) fn id(&self) -> u32 {
        self.0.id()
    }
}

impl Drop for Reaped {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
