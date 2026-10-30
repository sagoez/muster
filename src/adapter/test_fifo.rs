//! A FIFO for tests, made the same way on Linux and macOS.

use std::{ffi::CString, io, os::unix::ffi::OsStrExt, path::Path};

/// The mode a test FIFO is made with: owner read and write.
const FIFO_MODE: libc::mode_t = 0o600;

/// Makes a FIFO at `path`; rustix's `mknodat` is not available on macOS.
///
/// # Panics
/// Panics when the FIFO cannot be made, failing the test.
pub(crate) fn make_fifo(path: &Path) {
    let path = CString::new(path.as_os_str().as_bytes()).unwrap();
    // SAFETY: `path` is a valid nul-terminated C string that outlives the call.
    let made = unsafe { libc::mkfifo(path.as_ptr(), FIFO_MODE) };
    assert_eq!(made, 0, "mkfifo failed: {}", io::Error::last_os_error());
}
