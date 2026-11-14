//! Scratch directories for tests that bind Unix sockets.

use tempfile::{Builder, TempDir};

/// Short root for test sockets: macOS's long temp dir breaches `sun_path`.
const SHORT_TEMP_ROOT: &str = "/tmp";

/// A fresh scratch directory named from `prefix`, short enough to hold a
/// socket path.
pub fn short_scratch(prefix: &str) -> TempDir {
    Builder::new()
        .prefix(prefix)
        .tempdir_in(SHORT_TEMP_ROOT)
        .unwrap()
}
