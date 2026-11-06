//! What tests of a PTY's size share.

use super::PtySize;

/// A PTY size of `rows` by `cols`.
pub fn size(rows: u16, cols: u16) -> PtySize {
    PtySize::try_new(rows, cols).unwrap()
}
