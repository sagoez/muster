use muster_domain::value::OutputCursor;

use super::{log_line::LogLine, log_renderer::LogRenderer};

/// A [`LogRenderer`] that knows where in a process's output each byte it
/// renders sits, so a reader can tell a gap and start over after it.
pub struct PlacedRenderer {
    /// The rendering.
    renderer: LogRenderer,
    /// Where the next byte sits.
    next: OutputCursor,
}

impl PlacedRenderer {
    /// A renderer whose first byte sits at `at`.
    pub fn new(at: OutputCursor) -> Self {
        Self {
            renderer: LogRenderer::new(),
            next: at,
        }
    }

    /// Where the next byte sits.
    pub fn next(&self) -> OutputCursor {
        self.next
    }

    /// Renders `bytes`, the next piece of the output.
    pub fn advance(&mut self, bytes: &[u8]) {
        self.renderer.advance(bytes);
        self.next = self.next.advanced(bytes.len() as u64);
    }

    /// Starts over at `at`, past output never rendered: a gap rotation
    /// left, or a run this renderer did not see begin.
    pub fn skip_to(&mut self, at: OutputCursor) {
        self.renderer.restart();
        self.next = self.next.max(at);
    }

    /// Starts over here, as at a run start.
    pub fn restart(&mut self) {
        self.renderer.restart();
    }

    /// The lines finished since the last call, oldest first.
    pub fn take_lines(&mut self) -> Vec<LogLine> {
        self.renderer.take_lines()
    }

    /// Whether the line still being drawn was never redrawn in place.
    pub fn is_settled(&self) -> bool {
        self.renderer.is_settled()
    }

    /// What the line still being drawn shows past what was taken before;
    /// see [`LogRenderer::take_partial`].
    pub fn take_partial(&mut self) -> Option<LogLine> {
        self.renderer.take_partial()
    }

    /// The whole line still being drawn, unless it is blank.
    pub fn peek_partial(&self) -> Option<LogLine> {
        self.renderer.peek_partial()
    }

    /// Columns the line still being drawn spans.
    pub fn partial_width(&self) -> usize {
        self.renderer.partial_width()
    }

    /// Ends the line still being drawn as the end of the output does.
    pub fn finish(&mut self) {
        self.renderer.finish();
    }

    /// How many lines were dropped because rendering them panicked, since
    /// the last call.
    pub fn take_faults(&mut self) -> usize {
        self.renderer.take_faults()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A gap ends the line in progress, drops a sequence cut short and
    /// moves the cursor past it; a skip never moves the cursor back.
    #[test]
    fn a_gap_starts_the_rendering_over_past_it() {
        let mut renderer = PlacedRenderer::new(OutputCursor::new(100));
        renderer.advance(b"one\n\x1b[31mtwo\x1b[3");
        renderer.skip_to(OutputCursor::new(200));
        renderer.advance(b"1mthree\n");
        renderer.skip_to(OutputCursor::new(150));

        let lines: Vec<String> = renderer.take_lines().iter().map(LogLine::colored).collect();

        assert_eq!(lines, ["one", "\x1b[31mtwo\x1b[0m", "1mthree"]);
        assert_eq!(renderer.next(), OutputCursor::new(208));
    }
}
