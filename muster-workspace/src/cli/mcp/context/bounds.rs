//! The session context's byte bounds, which its listings and its entries
//! share.

/// The most bytes the context holds. Written whole in one write, it stays
/// within Linux's `PIPE_BUF` of 4096 bytes, so a reader of the hook's pipe
/// gets all of it or none, and under the 10,000 characters Claude Code keeps
/// of a hook's output.
pub const CONTEXT_MAX_BYTES: usize = 4_000;
/// The most bytes the open todos take, leaving the rest to the notes.
pub const TODOS_MAX_BYTES: usize = 1_400;
/// Fewer bytes than any entry takes as shown: once less room than this is
/// left, no further entry can fit.
pub const MIN_ENTRY_BYTES: usize = 16;

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::CONTEXT_MAX_BYTES;

    /// On Linux the whole context fits one atomic pipe write.
    #[test]
    fn the_context_fits_one_atomic_pipe_write() {
        const { assert!(CONTEXT_MAX_BYTES <= libc::PIPE_BUF) };
    }
}
