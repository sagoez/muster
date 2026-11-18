//! The session context's byte bounds, which its listings and its entries
//! share.

/// The most bytes the context holds: with its line end it fits one write
/// into the hook's empty pipe, which holds at least a page, so that write
/// never blocks and the watchdog cannot cut it; and it is under the 10,000
/// characters Claude Code keeps of a hook's output.
pub const CONTEXT_MAX_BYTES: usize = 4_000;
/// The most bytes the open todos take, leaving the rest to the notes.
pub const TODOS_MAX_BYTES: usize = 1_400;
/// The most bytes the held locks take, after the todos.
pub const LOCKS_MAX_BYTES: usize = 600;
/// Fewer bytes than any entry takes as shown: once less room than this is
/// left, no further entry can fit.
pub const MIN_ENTRY_BYTES: usize = 16;
/// The fewest bytes a note takes as shown, without its line end: a
/// one-letter agent label and key and an empty body, `- (by "a") "k":` and
/// an empty quote line. The session context's tests render that note.
pub const MIN_NOTE_BYTES: usize = 22;

#[cfg(test)]
mod tests {
    use super::CONTEXT_MAX_BYTES;

    /// The least an empty pipe holds: one page, on every system muster
    /// runs on.
    const SMALLEST_PIPE_BYTES: usize = 4_096;

    /// The context and its line end fit the smallest empty pipe whole.
    #[test]
    fn the_context_fits_an_empty_pipe() {
        const { assert!(CONTEXT_MAX_BYTES < SMALLEST_PIPE_BYTES) };
    }
}
