use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

use muster_domain::value::OutputCursor;

use super::{fault_budget::FaultBudget, tap_line::TapLine};
use crate::adapter::terminal::{
    escape::{LINE_FEED, RUN_RESET_AT, RUN_START},
    log_line::LogLine,
    log_renderer::LogRenderer,
    placed_renderer::PlacedRenderer,
};

/// Bytes of rendered lines a tap keeps, newest last.
pub const TAP_RING_BYTES: usize = 256 * 1024;
/// Columns an unfinished line may span and still be rendered whole for a
/// pattern on every chunk: a prompt is far shorter.
pub const CHEAP_PARTIAL_CELLS: usize = 1024;
/// How long a longer unfinished line waits to be rendered whole for a
/// pattern, so a line that never ends costs the loop one rendering of it
/// per this period, however many chunks land on it.
pub const PARTIAL_RECHECK: Duration = Duration::from_millis(100);

/// The output stream of one process as lines, for waits and readiness to be
/// tested against: renders each chunk as the next piece of one stream,
/// restarted at each run, and keeps the newest [`TAP_RING_BYTES`] of the
/// lines with the offset each began at. It outlives each run, so the last
/// lines of a run that ended stay readable.
///
/// Each byte is rendered once, as it arrives: a chunk costs time in
/// proportion to its own length, never to the length of the line it lands
/// on, so a line that never ends cannot make the service loop render it
/// again for every chunk. Only [`OutputTap::unfinished`] renders the line
/// in progress whole, for a caller that has a pattern to test it against,
/// and [`OutputTap::unfinished_for_pattern`] does so on a chunk only while
/// the line is short, leaving a longer one for [`OutputTap::unfinished_due`]
/// at most once per [`PARTIAL_RECHECK`].
pub struct OutputTap {
    /// The newest complete lines, oldest first.
    ring: VecDeque<TapLine>,
    /// The bytes of text the ring holds.
    ring_bytes: usize,
    /// The rendering of the stream, which knows where the next byte goes.
    renderer: PlacedRenderer,
    /// Where the line still being written began.
    partial_at: OutputCursor,
    /// Where the current run began, once one has.
    run_start: Option<OutputCursor>,
    /// Where the newest run in a seeded log began, by the reset the holder
    /// logs there.
    logged_run_start: Option<OutputCursor>,
    /// Lines this tap dropped because rendering them panicked.
    live_faults: FaultBudget,
    /// Lines reads of the log dropped, reported apart from the tap's own.
    read_faults: FaultBudget,
    /// Whether the cursor stands where the output really ends: a run began
    /// here, or the log's end was read.
    placed: bool,
    /// When a long unfinished line left untested for a pattern is due to be
    /// rendered whole and tested.
    recheck: Option<Instant>,
    /// The rendering of output the log could not hold, carried across its
    /// chunks so a line split between them is tested whole; gone once
    /// logged output or a new run comes.
    unlogged: Option<LogRenderer>,
}

impl OutputTap {
    /// An empty tap whose next byte is at `start`.
    pub fn new(start: OutputCursor) -> Self {
        Self {
            ring: VecDeque::new(),
            ring_bytes: 0,
            renderer: PlacedRenderer::new(start),
            partial_at: start,
            run_start: None,
            logged_run_start: None,
            live_faults: FaultBudget::default(),
            read_faults: FaultBudget::default(),
            placed: false,
            recheck: None,
            unlogged: None,
        }
    }

    /// Where the next byte goes: the end of the output seen.
    pub fn cursor(&self) -> OutputCursor {
        self.renderer.next()
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

    /// Takes `bytes`, the last of a log that starts at `at` on a line
    /// boundary, of output from runs this tap never saw begin, rendered as
    /// every reader renders the log. The cursor moves to their end and their
    /// lines are kept, and the last lines are those of the newest run whose
    /// start the bytes hold; but no run is claimed, as this service never
    /// saw one begin.
    pub fn seed_at(&mut self, at: OutputCursor, bytes: &[u8]) {
        if at >= self.cursor() {
            self.renderer.finish();
            self.keep_finished();
            self.renderer = PlacedRenderer::new(at);
            self.partial_at = at;
        }
        self.feed(Some(at), bytes);
        self.logged_run_start = bytes
            .windows(RUN_START.len())
            .rposition(|window| window == RUN_START)
            .map(|index| at.advanced((index + RUN_RESET_AT) as u64));
        self.placed = true;
    }

    /// Starts a new run here: the previous run's unfinished line ends, and
    /// the new run renders as on a fresh terminal.
    pub fn begin_run(&mut self) {
        self.renderer.restart();
        self.keep_finished();
        self.partial_at = self.cursor();
        self.run_start = Some(self.cursor());
        self.forget_last_run();
        self.placed = true;
    }

    /// Starts a new run at `start`, the offset its log says it began at,
    /// which every later positioned chunk counts from. The rendering starts
    /// over only where the run truly begins: at `start` when the tap stands
    /// there or before it. A tap already past `start` has rendered the run's
    /// first bytes, as a reconnect or a seeded log leaves it, and keeps what
    /// it drew of them; the cursor never moves back.
    pub fn begin_run_at(&mut self, start: OutputCursor) {
        if self.run_start == Some(start) {
            return;
        }
        if start >= self.cursor() {
            self.renderer.finish();
            self.keep_finished();
            self.renderer.skip_to(start);
            self.partial_at = start;
        }
        self.run_start = Some(start);
        self.forget_last_run();
        self.placed = true;
    }

    /// Takes one chunk of raw output, whose first byte is at `at` when the
    /// sender says so, and returns every line it completed. A chunk past the
    /// end of what was seen leaves a gap, which the rendering starts over
    /// after; one overlapping it has its repeated bytes skipped.
    pub fn feed(&mut self, at: Option<OutputCursor>, bytes: &[u8]) -> Vec<TapLine> {
        self.unlogged = None;
        let next = self.cursor();
        let bytes = match at {
            Some(at) if at > next => {
                self.renderer.finish();
                self.keep_finished();
                self.renderer.skip_to(at);
                self.partial_at = at;
                bytes
            },
            Some(at) if at < next => {
                let seen = usize::try_from(next.bytes_since(at)).unwrap_or(usize::MAX);
                bytes.get(seen..).unwrap_or_default()
            },
            _ => bytes,
        };
        let mut completed = Vec::new();
        for piece in bytes.split_inclusive(|byte| *byte == LINE_FEED) {
            self.renderer.advance(piece);
            let lines = self.keep_finished();
            if piece.ends_with(&[LINE_FEED]) || !lines.is_empty() {
                self.partial_at = self.cursor();
            }
            completed.extend(lines);
        }
        completed
    }

    /// The line still being written, if any, unless it only redrew the
    /// screen, so a prompt with no newline can match. It renders that line
    /// whole, so a caller asks only when it has a pattern to test.
    pub fn unfinished(&self) -> Option<TapLine> {
        self.renderer
            .peek_partial()
            .map(|line| self.tap_line(&line, self.partial_at))
    }

    /// The line still being written, for a pattern to test after a chunk at
    /// `now`, while it spans at most [`CHEAP_PARTIAL_CELLS`] columns. A
    /// longer one is not rendered now: it is due [`PARTIAL_RECHECK`] after
    /// the first chunk that left it untested, for [`Self::unfinished_due`].
    pub fn unfinished_for_pattern(&mut self, now: Instant) -> Option<TapLine> {
        if self.renderer.partial_width() <= CHEAP_PARTIAL_CELLS {
            self.recheck = None;
            return self.unfinished();
        }
        self.recheck.get_or_insert(now + PARTIAL_RECHECK);
        None
    }

    /// When a long unfinished line left untested is due, if one is.
    pub fn recheck_at(&self) -> Option<Instant> {
        self.recheck
    }

    /// The line still being written, rendered whole, once a long one left
    /// untested is due at `now`.
    pub fn unfinished_due(&mut self, now: Instant) -> Option<TapLine> {
        if self.recheck.is_none_or(|due| due > now) {
            return None;
        }
        self.recheck = None;
        self.unfinished()
    }

    /// The held lines that began at or after `since`, with the unfinished
    /// line last, and where the held output starts when `since` is older.
    pub fn scan_from(&self, since: OutputCursor) -> (Vec<TapLine>, Option<OutputCursor>) {
        let held_from = self.held_from();
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

    /// Counts `dropped` lines of this process's output that a read of its
    /// log could not render, to report apart from the tap's own, so reads
    /// never use up the report a fault in the live output is owed.
    pub fn note_read_faults(&mut self, dropped: usize) {
        self.read_faults.note(dropped);
    }

    /// How many lines this tap dropped because rendering them panicked, the
    /// first time any were in the current run; none after that until the
    /// next run begins, so a run is reported once however often it fails.
    pub fn take_fault_report(&mut self) -> Option<usize> {
        self.live_faults.note(self.renderer.take_faults());
        self.live_faults.take_report()
    }

    /// How many lines reads of the log dropped, counted and reported as
    /// [`Self::take_fault_report`] counts the tap's own.
    pub fn take_read_fault_report(&mut self) -> Option<usize> {
        self.read_faults.take_report()
    }

    /// Takes `bytes`, output the log could not hold, and returns the lines
    /// to test them as: those they finish and the one still open, while it
    /// is short enough to render on every chunk, each as beginning where
    /// the tap's output ends. The rendering carries on across such chunks,
    /// so a line split between them is tested whole, and leaves the tap and
    /// its cursor be. A line it drops counts as a log read's do.
    pub fn feed_unlogged(&mut self, bytes: &[u8]) -> Vec<TapLine> {
        let at = self.cursor();
        let renderer = self.unlogged.get_or_insert_with(LogRenderer::new);
        renderer.advance(bytes);
        let mut lines = renderer.take_lines();
        if renderer.partial_width() <= CHEAP_PARTIAL_CELLS {
            lines.extend(renderer.peek_partial());
        }
        self.read_faults.note(renderer.take_faults());
        lines.iter().map(|line| self.tap_line(line, at)).collect()
    }

    /// Begins a run for the fault reports, which report it afresh, and
    /// drops what was left of the last run's unlogged output.
    fn forget_last_run(&mut self) {
        self.live_faults.renew();
        self.read_faults.renew();
        self.unlogged = None;
    }

    /// Where the oldest line held begins, or where the next one will when
    /// none is.
    pub fn held_from(&self) -> OutputCursor {
        self.ring.front().map_or(self.partial_at, TapLine::at)
    }

    /// Keeps the lines the rendering finished, each as beginning where the
    /// line being written began, and returns them.
    fn keep_finished(&mut self) -> Vec<TapLine> {
        let lines: Vec<TapLine> = self
            .renderer
            .take_lines()
            .iter()
            .map(|line| self.tap_line(line, self.partial_at))
            .collect();
        for line in &lines {
            self.keep(line.clone());
        }
        lines
    }

    /// `line` as a tap line beginning at `at`.
    fn tap_line(&self, line: &LogLine, at: OutputCursor) -> TapLine {
        TapLine::builder().at(at).text(line.plain()).build()
    }

    /// The last `count` lines of the current run, oldest first, the
    /// unfinished one included.
    pub fn tail(&self, count: usize) -> Vec<String> {
        let start = self
            .run_start
            .or(self.logged_run_start)
            .unwrap_or(OutputCursor::new(0));
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
    use std::time::{Duration, Instant};

    use super::*;
    use crate::adapter::terminal::{line_drawing::MAX_LINE_CELLS, render_fault};

    /// Chunks of 1024 erases the endless-erase test feeds: a megabyte.
    const ERASE_CHUNKS: usize = 256;
    /// How long those chunks may take, many times what they need even in a
    /// debug build; the stream once took seconds per chunk.
    const ERASE_BOUND: Duration = Duration::from_secs(2);

    /// The text of each of `lines`.
    fn texts(lines: &[TapLine]) -> Vec<&str> {
        lines.iter().map(|line| line.text().as_str()).collect()
    }

    /// Lines are assembled across chunks with the offset each began at, and
    /// the unfinished one is there to test when asked for.
    #[test]
    fn assembles_rendered_lines_across_chunks_with_their_offsets() {
        let mut tap = OutputTap::new(OutputCursor::new(100));

        let first = tap.feed(None, b"\x1b[32mstart");
        let unfinished = tap.unfinished();
        let second = tap.feed(None, b"ing\r\nListening on :3000\nprompt> ");

        assert!(first.is_empty());
        assert_eq!(
            unfinished.map(|line| line.text().clone()).as_deref(),
            Some("start")
        );
        assert_eq!(texts(&second), ["starting", "Listening on :3000"]);
        assert_eq!(second[0].at(), OutputCursor::new(100));
        assert_eq!(second[1].at(), OutputCursor::new(115));
        let prompt = tap.unfinished().unwrap();
        assert_eq!(
            (prompt.text().as_str(), prompt.at()),
            ("prompt>", OutputCursor::new(134))
        );
        assert_eq!(tap.cursor(), OutputCursor::new(142));
    }

    /// A run that ends mid-escape leaves nothing of that to the next run,
    /// which renders from a fresh terminal; so does a gap.
    #[test]
    fn each_run_and_gap_starts_from_a_fresh_terminal() {
        let mut tap = OutputTap::new(OutputCursor::new(0));
        tap.begin_run();
        tap.feed(None, b"run 1\n\x1b]0;cut");

        tap.begin_run();
        let next_run = tap.feed(None, b"run 2\n");
        tap.feed(None, b"\x1b]0;cut");
        let after_gap = tap.feed(Some(OutputCursor::new(1000)), b"resumed\n");

        assert_eq!(texts(&next_run), ["run 2"]);
        assert_eq!(texts(&after_gap), ["resumed"]);
        assert_eq!(after_gap[0].at(), OutputCursor::new(1000));
    }

    /// A line that never ends while it is erased to the cursor again and
    /// again costs each chunk only its own bytes: this stream of 32 KiB of
    /// text followed by megabytes of `ESC [ 1 K` once kept the service loop
    /// busy for minutes, rendering the whole line again for every chunk and
    /// filling every column it erased. Its time is linear in the bytes fed,
    /// so the bound below is generous for any machine.
    #[test]
    fn an_endlessly_erased_line_costs_each_chunk_only_its_bytes() {
        let mut tap = OutputTap::new(OutputCursor::new(0));
        tap.feed(None, &vec![b'x'; 32 * 1024]);
        let erases = b"\x1b[1K".repeat(1024);
        let started = Instant::now();

        for _ in 0..ERASE_CHUNKS {
            assert!(tap.feed(None, &erases).is_empty());
        }

        assert!(
            started.elapsed() < ERASE_BOUND,
            "{:?} for {ERASE_CHUNKS} chunks",
            started.elapsed()
        );
        assert_eq!(tap.unfinished(), None, "the line reads as erased");
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

    /// Placing a run that began behind what the tap already rendered keeps
    /// the state the run's first bytes left, as a title still being written,
    /// rather than starting over mid-run.
    #[test]
    fn placing_a_run_behind_the_cursor_keeps_its_state() {
        let mut tap = OutputTap::new(OutputCursor::new(0));
        tap.seed_at(OutputCursor::new(40), b"\x1b]0;ti");

        tap.begin_run_at(OutputCursor::new(40));
        let lines = tap.feed(None, b"tle\x07frame\n");

        assert_eq!(texts(&lines), ["frame"]);
        assert_eq!(tap.run_start(), Some(OutputCursor::new(40)));
    }

    /// A seeded log's tail is the newest run its bytes hold the start of,
    /// found by the reset the holder logs there, and no run is claimed.
    #[test]
    fn a_seeded_tail_is_the_newest_logged_run() {
        let mut tap = OutputTap::new(OutputCursor::new(0));
        let mut bytes = b"old run\nleft open".to_vec();
        let reset = 100 + bytes.len() + RUN_RESET_AT;
        bytes.extend_from_slice(RUN_START);
        bytes.extend_from_slice(b"new 1\nnew 2\n");

        tap.seed_at(OutputCursor::new(100), &bytes);

        assert_eq!(
            tap.logged_run_start,
            Some(OutputCursor::new(reset as u64)),
            "the run starts at its reset, as the holder's log says"
        );
        assert_eq!(tap.tail(5), ["new 1", "new 2"]);
        assert_eq!(tap.run_start(), None, "no run is claimed");
        let (all, _) = tap.scan_from(OutputCursor::new(100));
        assert_eq!(texts(&all), ["old run", "left open", "new 1", "new 2"]);
    }

    /// Lines dropped because rendering them panicked are reported once per
    /// run, and again in the next run; those reads of the log dropped are
    /// counted apart, so a read reported first never uses up the report the
    /// live output's own fault is owed.
    #[test]
    fn dropped_lines_are_reported_once_per_run() {
        let mut tap = OutputTap::new(OutputCursor::new(0));
        tap.begin_run();
        assert_eq!(tap.take_fault_report(), None);

        tap.note_read_faults(2);
        let read = tap.take_read_fault_report();
        render_fault::arm();
        tap.feed(None, b"lost\nkept\n");
        let first = tap.take_fault_report();
        render_fault::arm();
        tap.feed(None, b"lost again\n");
        tap.note_read_faults(1);
        let same_run = (tap.take_fault_report(), tap.take_read_fault_report());
        tap.begin_run_at(OutputCursor::new(1000));
        tap.note_read_faults(1);
        let next_run = tap.take_read_fault_report();

        assert_eq!(read, Some(2));
        assert_eq!(first, Some(1), "a read never uses up the live report");
        assert_eq!(same_run, (None, None));
        assert_eq!(next_run, Some(1));
    }

    #[test]
    fn the_ring_and_an_endless_line_stay_bounded() {
        let mut tap = OutputTap::new(OutputCursor::new(0));
        let line = [b'x'; 1000];
        for _ in 0..TAP_RING_BYTES / 500 {
            tap.feed(None, &line);
            tap.feed(None, b"\n");
        }
        let endless = vec![b'y'; MAX_LINE_CELLS * 2 + 1];
        let cut = tap.feed(None, &endless);

        assert!(tap.ring_bytes <= TAP_RING_BYTES);
        assert_eq!(
            cut.iter().map(|line| line.text().len()).collect::<Vec<_>>(),
            [MAX_LINE_CELLS, MAX_LINE_CELLS]
        );
        assert_eq!(tap.unfinished().map(|line| line.text().len()), Some(1));
    }
}
