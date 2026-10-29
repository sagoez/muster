use std::collections::VecDeque;

use super::tap_line::TapLine;
use crate::{
    adapter::holder::{LINE_MAX_BYTES, strip_line},
    domain::value::OutputCursor,
};

/// Bytes of stripped lines a tap keeps, newest last.
pub const TAP_RING_BYTES: usize = 256 * 1024;

/// The output stream of one process as lines, for waits and readiness to be
/// tested against: strips each chunk, assembles lines, and keeps the newest
/// [`TAP_RING_BYTES`] of them with the offset each began at. It outlives each
/// run, so the last lines of a run that ended stay readable.
pub struct OutputTap {
    /// The newest complete lines, oldest first.
    ring: VecDeque<TapLine>,
    /// The bytes of text the ring holds.
    ring_bytes: usize,
    /// Raw bytes of the line still being written.
    partial: Vec<u8>,
    /// Where the line still being written began.
    partial_at: OutputCursor,
    /// Where the next byte goes.
    next: OutputCursor,
    /// Where the current run began, once one has.
    run_start: Option<OutputCursor>,
    /// Whether the cursor stands where the output really ends: a run began
    /// here, or the log's end was read.
    placed: bool,
}

impl OutputTap {
    /// An empty tap whose next byte is at `start`.
    pub fn new(start: OutputCursor) -> Self {
        Self {
            ring: VecDeque::new(),
            ring_bytes: 0,
            partial: Vec::new(),
            partial_at: start,
            next: start,
            run_start: None,
            placed: false,
        }
    }

    /// Where the next byte goes: the end of the output seen.
    pub fn cursor(&self) -> OutputCursor {
        self.next
    }

    /// Where the current run began, once one has.
    pub fn run_start(&self) -> Option<OutputCursor> {
        self.run_start
    }

    /// Whether the cursor stands where the output really ends, from a run
    /// this tap saw begin or the log's end it was seeded with.
    pub fn is_placed(&self) -> bool {
        self.placed
    }

    /// Takes `bytes`, the last of a log that starts at `at`, of output from
    /// runs this tap never saw begin: the cursor moves to their end and their
    /// lines are kept, but no run is claimed, so no run start is made up.
    pub fn seed_at(&mut self, at: OutputCursor, bytes: &[u8]) {
        self.feed(Some(at), bytes);
        self.placed = true;
    }

    /// Starts a new run here: the previous run's unfinished line ends.
    pub fn begin_run(&mut self) {
        self.flush_partial();
        self.run_start = Some(self.next);
        self.placed = true;
    }

    /// Starts a new run at `start`, the offset its log says it began at,
    /// which every later positioned chunk counts from. The current run placed
    /// again, as a reconnect does, keeps what was seen of it, and the cursor
    /// never moves back.
    pub fn begin_run_at(&mut self, start: OutputCursor) {
        if self.run_start == Some(start) {
            return;
        }
        self.flush_partial();
        if start > self.next {
            self.next = start;
            self.partial_at = start;
        }
        self.run_start = Some(start);
        self.placed = true;
    }

    /// Takes one chunk of raw output, whose first byte is at `at` when the
    /// sender says so, and returns the lines to test: every line it
    /// completed, then the unfinished line, if any, so a prompt with no
    /// newline can match. A chunk past the end of what was seen leaves a gap;
    /// one overlapping it has its repeated bytes skipped.
    pub fn feed(&mut self, at: Option<OutputCursor>, bytes: &[u8]) -> Vec<TapLine> {
        let bytes = match at {
            Some(at) if at > self.next => {
                self.flush_partial();
                self.next = at;
                self.partial_at = at;
                bytes
            },
            Some(at) if at < self.next => {
                let seen = usize::try_from(self.next.bytes_since(at)).unwrap_or(usize::MAX);
                bytes.get(seen..).unwrap_or_default()
            },
            _ => bytes,
        };
        let mut completed = Vec::new();
        for piece in bytes.split_inclusive(|byte| *byte == b'\n') {
            if self.partial.is_empty() {
                self.partial_at = self.next;
            }
            self.partial.extend_from_slice(piece);
            self.next = self.next.advanced(piece.len() as u64);
            if (piece.ends_with(b"\n") || self.partial.len() >= LINE_MAX_BYTES)
                && let Some(line) = self.flush_partial()
            {
                completed.push(line);
            }
        }
        if !self.partial.is_empty() {
            completed.push(
                TapLine::builder()
                    .at(self.partial_at)
                    .text(strip_line(&self.partial))
                    .build(),
            );
        }
        completed
    }

    /// The held lines that began at or after `since`, with the unfinished
    /// line last, and where the held output starts when `since` is older.
    pub fn scan_from(&self, since: OutputCursor) -> (Vec<TapLine>, Option<OutputCursor>) {
        let held_from = self.ring.front().map_or(self.partial_at, TapLine::at);
        let scanned_from = (since < held_from).then_some(held_from);
        let lines = self
            .ring
            .iter()
            .filter(|line| line.at() >= since)
            .cloned()
            .chain(self.unfinished().filter(|line| line.at() >= since))
            .collect();
        (lines, scanned_from)
    }

    /// The last `count` lines of the current run, oldest first, the
    /// unfinished one included.
    pub fn tail(&self, count: usize) -> Vec<String> {
        let start = self.run_start.unwrap_or(OutputCursor::new(0));
        let mut lines: Vec<String> = self
            .unfinished()
            .into_iter()
            .chain(
                self.ring
                    .iter()
                    .rev()
                    .take_while(|line| line.at() >= start)
                    .cloned(),
            )
            .take(count)
            .map(|line| line.text().clone())
            .collect();
        lines.reverse();
        lines
    }

    /// The line still being written, if any.
    fn unfinished(&self) -> Option<TapLine> {
        (!self.partial.is_empty()).then(|| {
            TapLine::builder()
                .at(self.partial_at)
                .text(strip_line(&self.partial))
                .build()
        })
    }

    /// Ends the line still being written, keeping it in the ring, and returns
    /// it.
    fn flush_partial(&mut self) -> Option<TapLine> {
        if self.partial.is_empty() {
            return None;
        }
        let line = TapLine::builder()
            .at(self.partial_at)
            .text(strip_line(&self.partial))
            .build();
        self.partial.clear();
        self.partial_at = self.next;
        self.keep(line.clone());
        Some(line)
    }

    /// Adds `line` to the ring, dropping the oldest lines past
    /// [`TAP_RING_BYTES`].
    fn keep(&mut self, line: TapLine) {
        self.ring_bytes += line.text().len();
        self.ring.push_back(line);
        while self.ring_bytes > TAP_RING_BYTES {
            let Some(dropped) = self.ring.pop_front() else {
                break;
            };
            self.ring_bytes -= dropped.text().len();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The text of each of `lines`.
    fn texts(lines: &[TapLine]) -> Vec<&str> {
        lines.iter().map(|line| line.text().as_str()).collect()
    }

    #[test]
    fn assembles_stripped_lines_across_chunks_with_their_offsets() {
        let mut tap = OutputTap::new(OutputCursor::new(100));

        let first = tap.feed(None, b"\x1b[32mstart");
        let second = tap.feed(None, b"ing\r\nListening on :3000\nprompt> ");

        assert_eq!(texts(&first), ["start"]);
        assert_eq!(texts(&second), [
            "starting",
            "Listening on :3000",
            "prompt>"
        ]);
        assert_eq!(second[0].at(), OutputCursor::new(100));
        assert_eq!(second[1].at(), OutputCursor::new(115));
        assert_eq!(tap.cursor(), OutputCursor::new(142));
    }

    #[test]
    fn a_progress_bar_collapses_to_its_last_state() {
        let mut tap = OutputTap::new(OutputCursor::new(0));

        let lines = tap.feed(None, b"10%\r55%\r100%\n");

        assert_eq!(texts(&lines), ["100%"]);
    }

    #[test]
    fn a_scan_returns_lines_from_the_cursor_and_says_when_it_is_too_old() {
        let mut tap = OutputTap::new(OutputCursor::new(50));
        tap.feed(None, b"one\ntwo\nthree");

        let (lines, scanned_from) = tap.scan_from(OutputCursor::new(54));
        let (all, too_old) = tap.scan_from(OutputCursor::new(0));

        assert_eq!(texts(&lines), ["two", "three"]);
        assert_eq!(scanned_from, None);
        assert_eq!(texts(&all), ["one", "two", "three"]);
        assert_eq!(too_old, Some(OutputCursor::new(50)));
    }

    #[test]
    fn a_positioned_chunk_skips_what_was_seen_and_marks_a_gap() {
        let mut tap = OutputTap::new(OutputCursor::new(0));
        tap.feed(Some(OutputCursor::new(0)), b"abc\n");

        let repeated = tap.feed(Some(OutputCursor::new(2)), b"c\nde\n");
        let after_gap = tap.feed(Some(OutputCursor::new(20)), b"x\n");

        assert_eq!(texts(&repeated), ["de"]);
        assert_eq!(after_gap[0].at(), OutputCursor::new(20));
        assert_eq!(tap.cursor(), OutputCursor::new(22));
    }

    #[test]
    fn the_tail_covers_only_the_current_run() {
        let mut tap = OutputTap::new(OutputCursor::new(0));
        tap.begin_run();
        tap.feed(None, b"old run\nunfinished");
        tap.begin_run();
        tap.feed(None, b"new 1\nnew 2\nnew 3\n");

        assert_eq!(tap.tail(2), ["new 2", "new 3"]);
        assert_eq!(tap.tail(10), ["new 1", "new 2", "new 3"]);
        assert_eq!(tap.run_start(), Some(OutputCursor::new(18)));
    }

    /// A run placed at its log offset counts from there, and positioned
    /// chunks continue it without a gap.
    #[test]
    fn a_run_begins_at_its_log_offset() {
        let mut tap = OutputTap::new(OutputCursor::new(0));
        tap.feed(None, b"counted by the service\n");

        tap.begin_run_at(OutputCursor::new(4096));
        let lines = tap.feed(Some(OutputCursor::new(4096)), b"Listening\n");

        assert_eq!(tap.run_start(), Some(OutputCursor::new(4096)));
        assert_eq!(lines[0].at(), OutputCursor::new(4096));
        assert_eq!(tap.cursor(), OutputCursor::new(4106));
        assert_eq!(tap.tail(5), ["Listening"]);
    }

    /// A log's tail seeds where the output ends and its lines, but claims no
    /// run, so no run start is made up for output this tap never saw begin.
    #[test]
    fn a_seeded_tail_places_the_cursor_without_a_run() {
        let mut tap = OutputTap::new(OutputCursor::new(0));
        assert!(!tap.is_placed());

        tap.seed_at(OutputCursor::new(4096), b"39999\n40000\n");

        assert!(tap.is_placed());
        assert_eq!(tap.run_start(), None);
        assert_eq!(tap.cursor(), OutputCursor::new(4108));
        let (lines, _) = tap.scan_from(OutputCursor::new(4096));
        assert_eq!(lines.len(), 2);
    }

    /// Placing the current run again keeps what was seen of it, and no run
    /// moves the cursor back.
    #[test]
    fn placing_a_run_never_moves_the_cursor_back() {
        let mut tap = OutputTap::new(OutputCursor::new(0));
        tap.begin_run_at(OutputCursor::new(40));
        tap.feed(Some(OutputCursor::new(40)), b"one\ntwo");

        tap.begin_run_at(OutputCursor::new(40));
        assert_eq!(tap.cursor(), OutputCursor::new(47));
        assert_eq!(tap.tail(5), ["one", "two"]);

        tap.begin_run_at(OutputCursor::new(10));
        assert_eq!(tap.cursor(), OutputCursor::new(47));
        assert_eq!(tap.run_start(), Some(OutputCursor::new(10)));
    }

    #[test]
    fn the_ring_and_an_endless_line_stay_bounded() {
        let mut tap = OutputTap::new(OutputCursor::new(0));
        let line = [b'x'; 1000];
        for _ in 0..TAP_RING_BYTES / 500 {
            tap.feed(None, &line);
            tap.feed(None, b"\n");
        }
        let endless = vec![b'y'; LINE_MAX_BYTES * 2];
        tap.feed(None, &endless);

        assert!(tap.ring_bytes <= TAP_RING_BYTES);
        assert!(tap.partial.len() < LINE_MAX_BYTES);
    }
}
