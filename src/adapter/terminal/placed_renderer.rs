use std::{collections::VecDeque, mem};

use super::{log_line::LogLine, log_renderer::LogRenderer, stream_marks::StreamMarks};
use crate::domain::value::OutputCursor;

/// A [`LogRenderer`] that knows where in a process's output each byte it
/// renders sits, and starts over at every run start it is told of, so what
/// one run left the terminal in never reaches the next.
pub struct PlacedRenderer {
    /// The rendering.
    renderer: LogRenderer,
    /// Where the next byte sits.
    next: OutputCursor,
    /// The run starts still ahead, oldest first.
    restarts: VecDeque<OutputCursor>,
}

impl PlacedRenderer {
    /// A renderer whose first byte sits at `at`, a point the output starts
    /// afresh from or the closest one a reader has.
    pub fn new(at: OutputCursor) -> Self {
        Self {
            renderer: LogRenderer::new(),
            next: at,
            restarts: VecDeque::new(),
        }
    }

    /// A renderer whose first byte sits at `at`, in the state `marks` say
    /// the stream is in there, restarting at each run start they record.
    pub fn placed(at: OutputCursor, marks: &StreamMarks) -> Self {
        let mut renderer = Self::new(at);
        renderer.restart_at(marks.run_starts());
        if marks.alternate_at(at) {
            renderer.renderer.enter_alternate();
        }
        renderer
    }

    /// Where the next byte sits.
    pub fn next(&self) -> OutputCursor {
        self.next
    }

    /// Notes run starts; only those not yet passed are kept. Merged with
    /// those known, so noting the same starts again costs their count.
    pub fn restart_at(&mut self, starts: impl IntoIterator<Item = OutputCursor>) {
        let mut incoming: Vec<OutputCursor> = starts
            .into_iter()
            .filter(|start| *start >= self.next)
            .collect();
        incoming.sort_unstable();
        let known = mem::take(&mut self.restarts);
        let mut known = known.into_iter().peekable();
        let mut incoming = incoming.into_iter().peekable();
        while let Some(start) = match (known.peek(), incoming.peek()) {
            (Some(old), Some(new)) if new < old => incoming.next(),
            (Some(_), _) => known.next(),
            (None, _) => incoming.next(),
        } {
            if self.restarts.back() != Some(&start) {
                self.restarts.push_back(start);
            }
        }
    }

    /// Renders `bytes`, the next piece of the output, starting over at each
    /// run start among them.
    pub fn advance(&mut self, mut bytes: &[u8]) {
        loop {
            while self
                .restarts
                .front()
                .is_some_and(|start| *start <= self.next)
            {
                self.restarts.pop_front();
                self.renderer.restart();
            }
            if bytes.is_empty() {
                return;
            }
            let room = self.restarts.front().map_or(bytes.len(), |start| {
                usize::try_from(start.bytes_since(self.next))
                    .unwrap_or(usize::MAX)
                    .min(bytes.len())
            });
            let (now, rest) = bytes.split_at(room);
            self.renderer.advance(now);
            self.next = self.next.advanced(now.len() as u64);
            bytes = rest;
        }
    }

    /// Starts over at `at`, past output never rendered: a gap rotation
    /// left, or a run this renderer did not see begin.
    pub fn skip_to(&mut self, at: OutputCursor) {
        self.renderer.restart();
        self.next = self.next.max(at);
        while self
            .restarts
            .front()
            .is_some_and(|start| *start < self.next)
        {
            self.restarts.pop_front();
        }
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

    /// The plain text of each line `renderer` finished.
    fn texts(renderer: &mut PlacedRenderer) -> Vec<String> {
        renderer.take_lines().iter().map(LogLine::plain).collect()
    }

    /// A run start inside a piece, at its edge or between pieces, restarts
    /// the rendering there and nowhere else.
    #[test]
    fn a_run_start_restarts_the_rendering_where_it_sits() {
        let first = b"one\n\x1b[?1049hTUI\n";
        let mut renderer = PlacedRenderer::new(OutputCursor::new(100));
        renderer.restart_at([OutputCursor::new(100 + first.len() as u64)]);

        let mut stream = first.to_vec();
        stream.extend_from_slice(b"two\n");
        renderer.advance(&stream);

        assert_eq!(texts(&mut renderer), ["one", "two"]);

        let mut split = PlacedRenderer::new(OutputCursor::new(0));
        split.restart_at([OutputCursor::new(first.len() as u64), OutputCursor::new(3)]);
        split.advance(&first[..2]);
        split.advance(&first[2..]);
        split.advance(b"\x1b[31");
        split.restart_at([OutputCursor::new(first.len() as u64 + 4)]);
        split.advance(b"mx\n");
        assert_eq!(
            texts(&mut split),
            ["one", "", "mx"],
            "a restart ends the line in progress and drops a sequence cut short"
        );
    }
}
