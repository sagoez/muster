/// What a step of a holder sockets folder's sweep was sweeping when it
/// failed, so its notice names what actually failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SweepTarget {
    /// The folder itself: refused as not private, unlisted, or, for the
    /// old folder, not removed once emptied.
    Folder,
    /// A launch's leftover spec or error file.
    HandoverFile,
    /// The socket of a holder that ended.
    Socket,
}
