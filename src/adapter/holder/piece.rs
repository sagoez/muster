use std::fs::File;

/// One generation's file and where it sits in the output stream.
pub(super) struct Piece {
    /// The open file; a rotation renaming it does not disturb the read.
    pub(super) file: File,
    /// The absolute offset of the first byte read from it.
    pub(super) start: u64,
    /// Bytes read from it, at most its length when it was opened.
    pub(super) len: u64,
    /// Bytes at the front of the file that fall before offset zero, which
    /// only a sidecar that disagrees with the files can leave.
    pub(super) skip: u64,
}
