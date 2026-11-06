/// What the mode scanner concluded after consuming one byte.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Observation {
    /// Nothing the emulator needs to act on.
    #[default]
    Nothing,
    /// This byte completes a switch to the alternate screen.
    EnterAlternate,
    /// This byte completes a switch back from the alternate screen.
    LeaveAlternate,
    /// This byte completes a scroll-region assignment (1-based margins).
    ScrollRegion {
        /// Top margin, at least 1.
        top: usize,
        /// Bottom margin, or `None` for the last screen line.
        bottom: Option<usize>,
    },
    /// This byte completes a full terminal reset.
    FullReset,
    /// This byte sets a tab stop at the cursor.
    SetTabStop,
    /// This byte clears the tab stop at the cursor.
    ClearTabStop,
    /// This byte clears every tab stop.
    ClearAllTabStops,
    /// This byte makes G1 the active charset.
    ShiftOut,
    /// This byte makes G0 the active charset.
    ShiftIn,
    /// This byte belongs to a sequence that ran past
    /// [`MAX_PENDING_BYTES`](super::bounded_parser::MAX_PENDING_BYTES), which
    /// is dropped up to its terminator; a parser fed the same stream drops it
    /// too.
    Dropped,
}
