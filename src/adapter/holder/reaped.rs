use std::process::Child;

/// Kills and collects a spawned child even when the test panics.
pub(super) struct Reaped(pub(super) Child);

impl Drop for Reaped {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
