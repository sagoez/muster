use std::time::Duration;

use nutype::nutype;

/// Interval between visible working-agent spinner frames.
pub(super) const ACTIVITY_FRAME_INTERVAL: Duration = Duration::from_millis(100);
/// Braille frames forming one compact terminal spinner cycle.
const ACTIVITY_FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Current frame of the transient working-agent indicator.
#[nutype(derive(Debug, Clone, Copy, PartialEq, Eq))]
pub(crate) struct ActivityFrame(usize);

impl ActivityFrame {
    /// Returns the first frame in the spinner cycle.
    pub(super) fn initial() -> Self {
        Self::new(0)
    }

    /// Advances to the next frame, wrapping at the end of the cycle.
    pub(super) fn next(self) -> Self {
        Self::new((self.into_inner() + 1) % ACTIVITY_FRAMES.len())
    }

    /// Returns the glyph for this frame.
    pub(super) fn glyph(self) -> &'static str {
        ACTIVITY_FRAMES[self.into_inner()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A full cycle returns to the initial frame.
    #[test]
    fn spinner_wraps_after_its_last_frame() {
        let mut frame = ActivityFrame::initial();
        for _ in ACTIVITY_FRAMES {
            frame = frame.next();
        }

        assert_eq!(frame, ActivityFrame::initial());
    }
}
