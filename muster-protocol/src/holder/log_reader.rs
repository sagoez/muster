//! Reading a process's log by output cursor: mapping the cursor through the
//! live and rotated files, finding the newest lines without reading the whole
//! log, rendering and filtering them, and capping what is returned.
//!
//! Every read renders the log as one stream, the way `muster logs` does, so
//! each path shows a line the same. The log holds no full-screen output and
//! a reset where each run starts, so a read that starts partway needs to
//! know nothing of what came before: it starts at a line boundary at most
//! [`RENDER_LOOKBACK_BYTES`] back, never inside a sequence, renders the
//! output before its start once, and shows only what follows.

use std::{
    collections::VecDeque,
    fs::File,
    io::{self, BufRead, BufReader, Read},
    mem,
    ops::ControlFlow,
    path::Path,
};

use muster_domain::value::OutputCursor;
use muster_terminal::{escape::LINE_FEED, log_line::LogLine, placed_renderer::PlacedRenderer};

use super::{
    error::HolderError, filter_state::FilterState, generation::Generation, kept::Kept,
    line_matcher::LineMatcher, log::rotated_path, log_base::LogBase, log_lines::LogLines,
    log_query::LogQuery, log_scan::LogScan, log_slice::LogSlice, logged_line::LoggedLine,
    span::Span, span_reader::SpanReader, strip::LINE_MAX_BYTES, visited_piece::VisitedPiece,
};

/// Most bytes of a resync's catch-up read from the log and sent at once, so
/// a long gap is replayed in pieces rather than held whole.
pub const CATCH_UP_CHUNK_BYTES: u64 = 64 * 1024;
/// The longest line a read returns, in characters; the rest is cut.
pub const LINE_MAX_CHARS: usize = 2000;
/// Continuation lines kept after each error line.
pub const ERROR_CONTEXT_LINES: usize = 12;
/// Separates non-adjacent error groups.
pub const GROUP_SEPARATOR: &str = "--";
/// Raw bytes of one line a backward scan renders to tell whether it counts;
/// the rest of a longer line is left out of that test.
pub(super) const RAW_LINE_KEEP_BYTES: usize = 64 * 1024;
/// Bytes read per step when scanning back from the end, and buffered when
/// reading forward.
pub(super) const BLOCK_BYTES: usize = 64 * 1024;
/// Most bytes before a read's start rendered, and not shown, so the read
/// starts in the state the whole stream's rendering is in there: a saved
/// cursor, the pen. Older output than this is taken to have left the
/// terminal as it found it.
pub const RENDER_LOOKBACK_BYTES: u64 = 256 * 1024;
/// Opens the note on a cut line, before the count of characters cut.
const CUT_OPEN: &str = " [... ";
/// Closes the note on a cut line.
const CUT_CLOSE: &str = " chars cut]";

/// Reads what `query` asks of the log at `log` and its rotated generation.
/// With a cursor, output after it, starting at the oldest kept byte when
/// rotation discarded what the cursor names; without one, the newest lines
/// that pass the filter. The returned cursor is the end of the log as read,
/// and `from` where the returned output starts: the first line kept when
/// the caps dropped older ones, otherwise where the read began.
///
/// # Errors
/// Returns [`HolderError::CursorAhead`] for a cursor past the end of the
/// log, or [`HolderError::Io`] when the files cannot be read.
pub fn read_log(log: &Path, query: &LogQuery) -> Result<LogSlice, HolderError> {
    let span = Span::open(log)?;
    let (start, rotated_away_bytes) = match query.since() {
        Some(since) if since > span.end() => {
            return Err(HolderError::CursorAhead {
                cursor: since,
                end: span.end(),
            });
        },
        Some(since) if since.max(span.oldest()) == span.end() => {
            return Ok(LogSlice::builder()
                .from(span.end())
                .cursor(span.end())
                .omitted_lines(0)
                .lines(Vec::new())
                .rotated_away_bytes(span.oldest().bytes_since(since))
                .unrendered_lines(0)
                .build());
        },
        Some(since) => (since.max(span.oldest()), span.oldest().bytes_since(since)),
        None => (span.newest_start(query)?, 0),
    };
    // The newest lines are found by testing each raw line on its own, which
    // cannot tell every line the stream's rendering drops. Reading from the
    // line boundary at the look-back before them lets the caps keep the
    // newest the stream shows; the lines before the estimate count as
    // omitted only if they are kept.
    let mut renderer = match query.since() {
        Some(_) => renderer_at(&span, start)?,
        None => PlacedRenderer::new(line_start_before(&span, start)?),
    };
    let begin = renderer.next();
    let mut kept = Kept::new(query);
    let mut filter = FilterState::new(query.filter());
    let mut reader = BufReader::with_capacity(BLOCK_BYTES, SpanReader::new(&span, begin));
    let mut offer = |lines: Vec<LogLine>, at: OutputCursor| {
        kept.count_from_now(at >= start);
        for line in &lines {
            filter.offer(cut(line.plain()), at, &mut kept);
        }
    };
    let mut line_at = begin;
    let mut at = begin;
    loop {
        let buffer = reader.fill_buf()?;
        if buffer.is_empty() {
            break;
        }
        let (taken, ended) = match buffer.iter().position(|byte| *byte == LINE_FEED) {
            Some(end) => (end + 1, true),
            None => (buffer.len(), false),
        };
        renderer.advance(&buffer[..taken]);
        reader.consume(taken);
        at = at.advanced(taken as u64);
        let lines = renderer.take_lines();
        let produced = !lines.is_empty();
        offer(lines, line_at);
        if ended || produced {
            line_at = at;
        }
    }
    renderer.finish();
    offer(renderer.take_lines(), line_at);
    let from = kept
        .first_at()
        .filter(|first| kept.omitted() > 0 || *first < start)
        .unwrap_or(start);
    Ok(LogSlice::builder()
        .from(from)
        .cursor(span.end())
        .omitted_lines(kept.omitted())
        .lines(kept.into_lines())
        .rotated_away_bytes(rotated_away_bytes)
        .unrendered_lines(renderer.take_faults())
        .build())
}

/// Every byte still kept of the log at `log` and its rotated generation, the
/// older first, exactly as written: the span [`read_log`] reads, unstripped,
/// for a reader that wants the raw stream. It comes with the cursor of its
/// first byte, so a reader knows where in the output it stands.
///
/// # Errors
/// Returns [`HolderError::Io`] when the files cannot be read.
pub fn read_log_stream(log: &Path) -> Result<(OutputCursor, Vec<u8>), HolderError> {
    let span = Span::open(log)?;
    let mut bytes = Vec::new();
    SpanReader::new(&span, span.oldest()).read_to_end(&mut bytes)?;
    Ok((span.oldest(), bytes))
}

/// Where `live`, a file opened from the live path of the log at `log`,
/// starts in the process's output: what the sidecar records for it, or
/// where the rotated file ends when the sidecar predates the rotation.
///
/// # Errors
/// Returns an [`io::Error`] when a file cannot be inspected.
pub fn live_start(log: &Path, live: &File) -> io::Result<OutputCursor> {
    let rotated = match File::open(rotated_path(log)) {
        Ok(file) => Some(Generation::of(&file)?),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    let resolved = LogBase::resolve(LogBase::read(log), Some(Generation::of(live)?), rotated);
    Ok(OutputCursor::new(resolved.offset()))
}

/// Where the output logged at `log` ends now.
///
/// # Errors
/// Returns [`HolderError::Io`] when the files cannot be inspected.
pub fn log_end(log: &Path) -> Result<OutputCursor, HolderError> {
    Ok(Span::open(log)?.end())
}

/// The raw bytes of the log at `log` from `from` up to `to`, at most `max`
/// of them, and where they start: at the oldest kept byte when rotation
/// discarded what `from` names, so the bound counts from there and a reader
/// sees the gap in where the bytes start.
///
/// # Errors
/// Returns [`HolderError::Io`] when the files cannot be read.
pub fn read_log_range(
    log: &Path,
    from: OutputCursor,
    to: OutputCursor,
    max: u64,
) -> Result<(OutputCursor, Vec<u8>), HolderError> {
    let span = Span::open(log)?;
    let start = from.max(span.oldest());
    let length = to
        .min(span.end())
        .min(start.advanced(max))
        .bytes_since(start);
    let mut bytes = Vec::new();
    SpanReader::new(&span, start)
        .take(length)
        .read_to_end(&mut bytes)?;
    Ok((start, bytes))
}

/// The last bytes of the log at `log`, at most `max`, and where they start:
/// just past the first line feed among them, so they begin on a line
/// boundary, unless they hold all the log keeps or no line feed at all.
///
/// # Errors
/// Returns [`HolderError::Io`] when the files cannot be read.
pub fn read_log_tail(log: &Path, max: u64) -> Result<(OutputCursor, Vec<u8>), HolderError> {
    let span = Span::open(log)?;
    let from = span
        .oldest()
        .max(OutputCursor::new(span.end().offset().saturating_sub(max)));
    let mut bytes = Vec::new();
    SpanReader::new(&span, from)
        .take(span.end().bytes_since(from))
        .read_to_end(&mut bytes)?;
    if from == span.oldest() {
        return Ok((from, bytes));
    }
    let boundary = bytes
        .iter()
        .position(|byte| *byte == LINE_FEED)
        .map_or(0, |feed| feed + 1);
    Ok((from.advanced(boundary as u64), bytes.split_off(boundary)))
}

/// The first line of the log at `log` that begins from `from` up to `until`
/// and `matcher` matches, with where the scan began.
///
/// # Errors
/// Returns [`HolderError::Io`] when the log cannot be read.
pub fn first_match(
    log: &Path,
    from: OutputCursor,
    until: OutputCursor,
    matcher: &LineMatcher,
) -> Result<LogScan, HolderError> {
    let mut matched = None;
    let (began, unrendered) = visit_lines(log, from, until, |piece| {
        let Some((index, text)) = piece
            .texts()
            .iter()
            .enumerate()
            .find(|(_, text)| matcher.is_match(text))
        else {
            return ControlFlow::Continue(());
        };
        matched = Some(
            LoggedLine::builder()
                .at(piece.began(index))
                .text(text.clone())
                .build(),
        );
        ControlFlow::Break(())
    })?;
    Ok(LogScan::builder()
        .from(began)
        .matched(matched)
        .unrendered_lines(unrendered)
        .build())
}

/// The last `count` lines of the log at `log` that begin from `from` up to
/// `until`, stripped, oldest first, read from at most `max` bytes before
/// `until`; a line cut by that bound is left out.
///
/// # Errors
/// Returns [`HolderError::Io`] when the log cannot be read.
pub fn last_lines(
    log: &Path,
    from: OutputCursor,
    until: OutputCursor,
    count: usize,
    max: u64,
) -> Result<LogLines, HolderError> {
    if count == 0 {
        return Ok(LogLines::default());
    }
    let mut lines = VecDeque::with_capacity(count);
    let bound = from.max(OutputCursor::new(until.offset().saturating_sub(max)));
    let mut cut = bound > from;
    let start = if cut {
        OutputCursor::new(bound.offset() - 1)
    } else {
        bound
    };
    let (_, unrendered) = visit_lines(log, start, until, |piece| {
        if mem::take(&mut cut) && piece.at() == start {
            return ControlFlow::Continue(());
        }
        for text in piece.texts() {
            if lines.len() == count {
                lines.pop_front();
            }
            lines.push_back(text.clone());
        }
        ControlFlow::Continue(())
    })?;
    Ok(LogLines::builder()
        .lines(lines.into())
        .unrendered_lines(unrendered)
        .build())
}

/// Hands `visit` each raw line of the log at `log` that begins from `from` up
/// to `until`, with where it began, the text of the lines its rendering, as
/// part of the whole stream, finished, and where the first of those began;
/// reading at most [`CATCH_UP_CHUNK_BYTES`] at a time. A line cut at
/// [`LINE_MAX_BYTES`], at a gap rotation left, or at `until` is handed as it
/// stands, and the last one with the text of the line it leaves unfinished.
/// Once `visit` breaks it is never called again. Returns where the read
/// began: `from`, or past the oldest byte kept when that is newer, and how
/// many lines rendering dropped.
///
/// # Errors
/// Returns [`HolderError::Io`] when the first read fails; a later failure
/// ends the lines early.
pub fn visit_lines(
    log: &Path,
    from: OutputCursor,
    until: OutputCursor,
    mut visit: impl FnMut(&VisitedPiece<'_>) -> ControlFlow<()>,
) -> Result<(OutputCursor, usize), HolderError> {
    let mut renderer = {
        let span = Span::open(log)?;
        renderer_at(&span, from.max(span.oldest()))?
    };
    let start = renderer.next().max(from);
    let mut began = None;
    let mut next = start;
    let mut line: Vec<u8> = Vec::new();
    let mut line_at = start;
    let mut drawn_from = start;
    while next < until {
        let (at, bytes) = match read_log_range(log, next, until, CATCH_UP_CHUNK_BYTES) {
            Ok(read) => read,
            Err(error) if began.is_none() => return Err(error),
            Err(_) => break,
        };
        began.get_or_insert(at);
        if bytes.is_empty() {
            break;
        }
        if at > next {
            let (flow, _) = hand_line(
                &mut visit,
                &mut renderer,
                &mut line,
                (line_at, drawn_from),
                true,
            );
            if flow.is_break() {
                return Ok((began.unwrap_or(start), renderer.take_faults()));
            }
            renderer.skip_to(at);
            drawn_from = at;
        }
        let mut cursor = at;
        for piece in bytes.split_inclusive(|byte| *byte == LINE_FEED) {
            if line.is_empty() {
                line_at = cursor;
            }
            line.extend_from_slice(piece);
            cursor = cursor.advanced(piece.len() as u64);
            let ended = piece.ends_with(&[LINE_FEED]);
            if !ended && line.len() < LINE_MAX_BYTES {
                continue;
            }
            let (flow, produced) = hand_line(
                &mut visit,
                &mut renderer,
                &mut line,
                (line_at, drawn_from),
                false,
            );
            if flow.is_break() {
                return Ok((began.unwrap_or(start), renderer.take_faults()));
            }
            if ended || produced {
                drawn_from = cursor;
            }
        }
        next = cursor;
    }
    let _ = hand_line(
        &mut visit,
        &mut renderer,
        &mut line,
        (line_at, drawn_from),
        true,
    );
    Ok((began.unwrap_or(start), renderer.take_faults()))
}

/// Renders `line`, which began at the first of `(at, drawn_from)` while the
/// line its rendering draws began at the second, and hands it to `visit`
/// with the text of the lines it finished, then empties it; `ending`
/// finishes the line it leaves unfinished too. Nothing is handed when there
/// is neither a byte nor a line. Returns what `visit` asked, and whether a
/// line was finished.
fn hand_line(
    visit: &mut impl FnMut(&VisitedPiece<'_>) -> ControlFlow<()>,
    renderer: &mut PlacedRenderer,
    line: &mut Vec<u8>,
    (at, drawn_from): (OutputCursor, OutputCursor),
    ending: bool,
) -> (ControlFlow<()>, bool) {
    renderer.advance(line);
    if ending {
        renderer.finish();
    }
    let texts: Vec<String> = renderer.take_lines().iter().map(LogLine::plain).collect();
    if line.is_empty() && texts.is_empty() {
        return (ControlFlow::Continue(()), false);
    }
    let flow = visit(
        &VisitedPiece::builder()
            .at(at)
            .bytes(line)
            .line_at(drawn_from)
            .texts(&texts)
            .build(),
    );
    line.clear();
    (flow, !texts.is_empty())
}

/// A renderer for a read of the output `span` holds from `start`, in the
/// state the whole stream's rendering is in there: it renders, without
/// showing, the output from where [`look_back`] begins up to `start`, and
/// ends there the line `start` falls inside, so the read shows from
/// `start`. The sequence state `start` falls inside is kept, so a read
/// never starts mid-escape. The look-back is read once, the rendering
/// starting over where it learns the read begins later.
///
/// # Errors
/// Returns an [`io::Error`] when the files cannot be read.
fn renderer_at(span: &Span, start: OutputCursor) -> io::Result<PlacedRenderer> {
    let mut drawn: Option<(OutputCursor, PlacedRenderer)> = None;
    let begin = look_back(span, start, |begin, _, piece| {
        let (_, renderer) = match &mut drawn {
            Some(drawn) if drawn.0 == begin => drawn,
            _ => drawn.insert((begin, PlacedRenderer::new(begin))),
        };
        renderer.advance(piece);
        renderer.take_lines();
        ControlFlow::Continue(())
    })?;
    let mut renderer = drawn.map_or_else(|| PlacedRenderer::new(begin), |(_, drawn)| drawn);
    renderer.finish();
    renderer.take_lines();
    renderer.take_faults();
    if renderer.next() < start {
        renderer.skip_to(start);
    }
    Ok(renderer)
}

/// Where a read from `start` of the output `span` holds begins rendering,
/// as [`look_back`] finds it, reading only up to the line start it finds.
///
/// # Errors
/// Returns an [`io::Error`] when the files cannot be read.
fn line_start_before(span: &Span, start: OutputCursor) -> io::Result<OutputCursor> {
    look_back(span, start, |_, on_line, _| {
        if on_line {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    })
}

/// Reads, once, the output `span` holds between the look-back bound before
/// `start` and `start`, and hands `take` each piece with where the read
/// begins rendering as far as is known, and whether that is a line start.
/// The look-back bound lies no more than [`RENDER_LOOKBACK_BYTES`] back;
/// the read begins at the oldest byte kept, where the log starts each
/// generation on a line, or just past the first line feed after the bound,
/// so it never begins inside a sequence, or at the bound for a line longer
/// than the look-back. A piece handed under a later beginning starts over.
/// Reading stops when `take` breaks. Returns where the read begins.
///
/// # Errors
/// Returns an [`io::Error`] when the files cannot be read.
fn look_back(
    span: &Span,
    start: OutputCursor,
    mut take: impl FnMut(OutputCursor, bool, &[u8]) -> ControlFlow<()>,
) -> io::Result<OutputCursor> {
    let bound = span.oldest().max(OutputCursor::new(
        start.offset().saturating_sub(RENDER_LOOKBACK_BYTES),
    ));
    let mut begin = bound;
    let mut on_line = bound == span.oldest();
    if on_line && take(begin, on_line, &[]).is_break() {
        return Ok(begin);
    }
    let mut reader = SpanReader::new(span, bound).take(start.bytes_since(bound));
    let mut block = vec![0; BLOCK_BYTES];
    let mut at = bound;
    loop {
        let read = reader.read(&mut block)?;
        if read == 0 {
            return Ok(begin);
        }
        let mut piece = &block[..read];
        if !on_line && let Some(index) = piece.iter().position(|byte| *byte == LINE_FEED) {
            on_line = true;
            begin = at.advanced(index as u64 + 1);
            piece = &piece[index + 1..];
        }
        at = at.advanced(read as u64);
        if take(begin, on_line, piece).is_break() {
            return Ok(begin);
        }
    }
}

/// `text` cut to [`LINE_MAX_CHARS`] with a note of how many characters went.
fn cut(text: String) -> String {
    let length = text.chars().count();
    if length <= LINE_MAX_CHARS {
        return text;
    }
    let kept: String = text.chars().take(LINE_MAX_CHARS).collect();
    format!("{kept}{CUT_OPEN}{}{CUT_CLOSE}", length - LINE_MAX_CHARS)
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        ops::Range,
        path::PathBuf,
        time::{Duration, Instant},
    };

    use muster_domain::{
        process::{LogPattern, PatternKind, PatternText},
        value::OutputCursor,
    };
    use muster_terminal::{
        line_drawing::{MAX_COLUMN, MAX_LINE_CELLS},
        render_fault,
    };
    use tempfile::TempDir;

    use super::*;
    use crate::holder::{
        byte_limit::ByteLimit, gated_log::GatedLog, line_filter::LineFilter, line_limit::LineLimit,
        line_matcher::LineMatcher, log::rotated_path, log_fixture::Fixture,
        screen_gate::FULL_SCREEN_LINE,
    };

    /// A limit of `count` lines.
    fn lines(count: usize) -> LineLimit {
        LineLimit::try_new(count).unwrap()
    }

    /// A filter keeping lines that match `text`, read as `kind`.
    fn grep(text: &str, kind: PatternKind) -> LineFilter {
        LineFilter::Grep(
            LineMatcher::compile(
                &LogPattern::builder()
                    .text(PatternText::try_new(text).unwrap())
                    .kind(kind)
                    .build(),
            )
            .unwrap(),
        )
    }

    /// One numbered line for each of `range`.
    fn numbered(range: Range<usize>) -> String {
        range.map(|line| format!("line {line}\n")).collect()
    }

    #[test]
    fn a_missing_log_reads_as_empty_at_zero() {
        let scratch = TempDir::new().unwrap();
        let slice = read_log(
            &scratch.path().join("none.log"),
            &LogQuery::builder().build(),
        )
        .unwrap();
        assert_eq!(slice.from(), OutputCursor::new(0));
        assert_eq!(slice.cursor(), OutputCursor::new(0));
        assert!(slice.lines().is_empty());
    }

    /// Without a cursor the newest lines come back; the cursor is the end, and
    /// reading from it returns only what was written after.
    #[test]
    fn the_tail_then_only_new_output() {
        let mut fixture = Fixture::default();
        fixture.write(b"l1\nl2\nl3\nl4\nl5\n");
        let tail = fixture.read(&LogQuery::builder().lines(lines(3)).build());
        assert_eq!(tail.lines(), &["l3", "l4", "l5"]);
        assert_eq!(tail.from(), OutputCursor::new(6));
        assert_eq!(tail.cursor(), OutputCursor::new(15));
        assert_eq!(tail.omitted_lines(), 0);

        let since = fixture.write(b"l6\nl7\n");
        assert_eq!(since, tail.cursor());
        let next = fixture.read(&LogQuery::builder().since(Some(tail.cursor())).build());
        assert_eq!(next.lines(), &["l6", "l7"]);
        assert_eq!(next.from(), tail.cursor());

        let nothing = fixture.read(&LogQuery::builder().since(Some(next.cursor())).build());
        assert!(nothing.lines().is_empty());
        assert_eq!(nothing.cursor(), next.cursor());
    }

    /// A line of output that reads `--` is a line like any other, never
    /// taken for the separator between error groups, so dropping it to keep
    /// the newest lines loses nothing else.
    #[test]
    fn a_line_reading_like_a_separator_is_a_line() {
        let mut fixture = Fixture::default();
        fixture.write(b"a\n--\nb\n");

        let newest = fixture.read(&LogQuery::builder().lines(lines(1)).build());
        let two = fixture.read(&LogQuery::builder().lines(lines(2)).build());
        let all = fixture.read(
            &LogQuery::builder()
                .since(Some(OutputCursor::new(0)))
                .build(),
        );

        assert_eq!(newest.lines(), &["b"]);
        assert_eq!(two.lines(), &["--", "b"]);
        assert_eq!(all.lines(), &["a", "--", "b"]);
    }

    /// Output a prompt left without a line feed is read as a line.
    #[test]
    fn a_trailing_partial_line_is_returned() {
        let mut fixture = Fixture::default();
        fixture.write(b"first\nName? ");
        let slice = fixture.read(&LogQuery::builder().build());
        assert_eq!(slice.lines(), &["first", "Name?"]);
    }

    #[test]
    fn escapes_are_stripped_and_progress_collapses() {
        let mut fixture = Fixture::default();
        fixture.write(b"\x1b[32mready\x1b[0m\r\n10%\r50%\r100%\r\n");
        let slice = fixture.read(&LogQuery::builder().build());
        assert_eq!(slice.lines(), &["ready", "100%"]);
    }

    /// A status footer redrawn after every line and the room made for it
    /// read as nothing, so the newest lines asked for are all real ones.
    #[test]
    fn footer_redraws_are_left_out_of_the_newest_lines() {
        let mut fixture = Fixture::default();
        let footer = b"\x1b[0K\r\n\x1b[0G\x1b[0K\r\n\x1b[0G\x1b[2A\x1b[s\x1b7\x1b[30;0H\
            \x1b[0Kw Enable Watch   d Detach\x1b[u\x1b8";
        for line in ["one", "two", "three", "four"] {
            fixture.write(format!("web-1  | {line}\r\n").as_bytes());
            fixture.write(footer);
        }

        let slice = fixture.read(&LogQuery::builder().lines(lines(3)).build());

        assert_eq!(slice.lines(), &[
            "web-1  | two",
            "web-1  | three",
            "web-1  | four"
        ]);
    }

    /// A cursor keeps working across rotations; one older than the oldest
    /// kept byte reads from there and says how much rotation discarded.
    #[test]
    fn cursors_map_across_rotation() {
        let mut fixture = Fixture::with_cap(12);
        let first = fixture.write(b"aaaa\nbbbb\n");
        let second = fixture.write(b"cccc\ndddd\n");
        fixture.write(b"eeee\n");
        assert!(fs::metadata(rotated_path(fixture.path())).is_ok());

        let from_second = fixture.read(&LogQuery::builder().since(Some(second)).build());
        assert_eq!(from_second.lines(), &["cccc", "dddd", "eeee"]);
        assert_eq!(from_second.rotated_away_bytes(), 0);

        let from_first = fixture.read(&LogQuery::builder().since(Some(first)).build());
        assert_eq!(from_first.lines(), &["cccc", "dddd", "eeee"]);
        assert_eq!(from_first.from(), second);
        assert_eq!(from_first.rotated_away_bytes(), 10);

        let mid_line = fixture.read(&LogQuery::builder().since(Some(second.advanced(2))).build());
        assert_eq!(mid_line.lines(), &["cc", "dddd", "eeee"]);
    }

    /// The raw stream is both generations, the older first, byte for byte,
    /// escapes and invalid UTF-8 included; a missing log reads as nothing.
    #[test]
    fn the_raw_stream_is_both_generations_unchanged() {
        let mut fixture = Fixture::with_cap(12);
        assert!(read_log_stream(fixture.path()).unwrap().1.is_empty());
        fixture.write(b"\x1b[31maaaa\n");
        fixture.write(b"live \xff\n");

        assert_eq!(
            read_log_stream(fixture.path()).unwrap().1,
            b"\x1b[31maaaa\nlive \xff\n"
        );
    }

    /// A scan finds the first line its pattern matches that begins before
    /// its bound, at its offset, and one from a cursor rotation discarded
    /// says it began at the oldest byte kept.
    #[test]
    fn a_scan_finds_the_first_match_before_its_bound_and_where_it_began() {
        let mut fixture = Fixture::with_cap(12);
        let first = fixture.write(b"aaaa\nbbbb\n");
        let second = fixture.write(b"cccc\ndddd\n");
        let last = fixture.write(b"dd2\n");
        let matcher = LineMatcher::of_text("d", PatternKind::Substring).unwrap();

        let rotated = first_match(fixture.path(), first, last.advanced(4), &matcher).unwrap();
        let bounded = first_match(fixture.path(), second, second.advanced(5), &matcher).unwrap();
        let kept = first_match(
            fixture.path(),
            second.advanced(6),
            last.advanced(4),
            &matcher,
        )
        .unwrap();

        assert_eq!(rotated.from(), second);
        let found = rotated.matched().clone().unwrap();
        assert_eq!(
            (found.at(), found.text().as_str()),
            (second.advanced(5), "dddd")
        );
        assert_eq!(bounded.from(), second);
        assert_eq!(*bounded.matched(), None);
        assert_eq!(kept.from(), second.advanced(6));
        let found = kept.matched().clone().unwrap();
        assert_eq!(
            (found.at(), found.text().as_str()),
            (second.advanced(6), "ddd")
        );
    }

    /// The last lines of a span come back oldest first, across both
    /// generations, and a line the byte bound cuts is left out.
    #[test]
    fn the_last_lines_span_both_generations_and_leave_out_a_cut_line() {
        let mut fixture = Fixture::with_cap(12);
        let first = fixture.write(b"aaaa\nbbbb\n");
        fixture.write(b"cccc\ndddd\n");
        let end = fixture.write(b"eeee\n").advanced(5);

        let whole = last_lines(fixture.path(), first, end, 2, BLOCK_BYTES as u64).unwrap();
        let cut = last_lines(fixture.path(), first, end, 10, 7).unwrap();
        let aligned = last_lines(fixture.path(), first, end, 10, 10).unwrap();

        assert_eq!(*whole.lines(), ["dddd", "eeee"]);
        assert_eq!(*cut.lines(), ["eeee"]);
        assert_eq!(*aligned.lines(), ["dddd", "eeee"]);
    }

    /// The tail spans the rotated and live files as one stream.
    #[test]
    fn the_tail_spans_both_generations() {
        let mut fixture = Fixture::with_cap(12);
        fixture.write(b"aaaa\nbb");
        fixture.write(b"bb\ncccc\n");
        let slice = fixture.read(&LogQuery::builder().lines(lines(3)).build());
        assert_eq!(slice.lines(), &["aaaa", "bbbb", "cccc"]);
        assert_eq!(slice.from(), OutputCursor::new(0));
        assert_eq!(slice.cursor(), OutputCursor::new(15));
    }

    #[test]
    fn a_cursor_past_the_end_is_refused() {
        let mut fixture = Fixture::default();
        fixture.write(b"one\n");
        let error = read_log(
            fixture.path(),
            &LogQuery::builder()
                .since(Some(OutputCursor::new(99)))
                .build(),
        )
        .unwrap_err();
        assert!(matches!(error, HolderError::CursorAhead { .. }), "{error}");
    }

    /// Grep keeps the newest matching lines, found by scanning back past
    /// blocks of lines that do not match.
    #[test]
    fn grep_finds_the_newest_matches_far_back() {
        let mut fixture = Fixture::default();
        fixture.write(b"GET /a 500\nGET /b 500\n");
        fixture.write(numbered(0..20_000).as_bytes());
        fixture.write(b"GET /c 500\n");
        let substring = fixture.read(
            &LogQuery::builder()
                .lines(lines(2))
                .filter(grep("GET /", PatternKind::Substring))
                .build(),
        );
        assert_eq!(substring.lines(), &["GET /b 500", "GET /c 500"]);
        assert_eq!(substring.from(), OutputCursor::new(11));

        let regex = fixture.read(
            &LogQuery::builder()
                .filter(grep(r"^line 1999\d$", PatternKind::Regex))
                .build(),
        );
        assert_eq!(regex.lines().len(), 10);
        assert_eq!(regex.lines()[0], "line 19990");
    }

    /// Errors-only keeps each error with its trace, separates groups that are
    /// not adjacent and stops a group at its context limit.
    #[test]
    fn errors_only_keeps_whole_traces() {
        let mut fixture = Fixture::default();
        let mut text = String::from(
            "GET / 200\nError: boom\n    at a (x.js:1)\n    at b (x.js:2)\nGET / 200\n",
        );
        text.push_str(
            "Traceback (most recent call last):\n  File \"a.py\", line 1\nValueError: bad\nok\n",
        );
        text.push_str("panic: deep\n");
        for frame in 0..20 {
            text.push_str(&format!("\tat frame{frame}\n"));
        }
        fixture.write(text.as_bytes());

        let slice = fixture.read(&LogQuery::builder().filter(LineFilter::ErrorsOnly).build());
        let mut expected = vec![
            "Error: boom",
            "    at a (x.js:1)",
            "    at b (x.js:2)",
            "--",
            "Traceback (most recent call last):",
            "  File \"a.py\", line 1",
            "ValueError: bad",
            "--",
            "panic: deep",
        ];
        let frames: Vec<String> = (0..ERROR_CONTEXT_LINES)
            .map(|frame| format!("\tat frame{frame}"))
            .collect();
        expected.extend(frames.iter().map(String::as_str));
        assert_eq!(slice.lines(), &expected);

        let newest = fixture.read(
            &LogQuery::builder()
                .lines(lines(1))
                .filter(LineFilter::ErrorsOnly)
                .build(),
        );
        assert_eq!(newest.lines(), &["\tat frame11"]);
        assert_eq!(newest.omitted_lines(), ERROR_CONTEXT_LINES);
    }

    /// Past the line or byte cap the oldest lines go and are counted, and a
    /// separator is never left leading.
    #[test]
    fn caps_drop_the_oldest_lines() {
        let mut fixture = Fixture::default();
        let start = fixture.write(numbered(0..10).as_bytes());
        let by_lines = fixture.read(
            &LogQuery::builder()
                .since(Some(start))
                .lines(lines(4))
                .build(),
        );
        assert_eq!(by_lines.lines(), &["line 6", "line 7", "line 8", "line 9"]);
        assert_eq!(by_lines.omitted_lines(), 6);
        assert_eq!(
            by_lines.from(),
            start.advanced(numbered(0..6).len() as u64),
            "from is the first line returned"
        );

        let by_bytes = fixture.read(
            &LogQuery::builder()
                .since(Some(start))
                .max_bytes(ByteLimit::try_new(14).unwrap())
                .build(),
        );
        assert_eq!(by_bytes.lines(), &["line 8", "line 9"]);
        assert_eq!(by_bytes.omitted_lines(), 8);
        assert_eq!(by_bytes.from(), start.advanced(numbered(0..8).len() as u64));
        let whole = fixture.read(&LogQuery::builder().since(Some(start)).build());
        assert_eq!(whole.from(), start, "nothing cut, from is where it began");

        fixture.write(b"error one\nfine\nerror two\n");
        let errors = fixture.read(
            &LogQuery::builder()
                .filter(LineFilter::ErrorsOnly)
                .lines(lines(2))
                .build(),
        );
        assert_eq!(errors.lines(), &["error one", "--", "error two"]);
        let one_error = fixture.read(
            &LogQuery::builder()
                .since(Some(start))
                .filter(LineFilter::ErrorsOnly)
                .lines(lines(1))
                .build(),
        );
        assert_eq!(one_error.lines(), &["error two"]);
        assert_eq!(one_error.omitted_lines(), 1);
    }

    /// A line past the character limit is cut with a note, and one past
    /// what a line holds is cut into lines of its own, as `muster logs`
    /// shows it, never held whole.
    #[test]
    fn long_lines_are_cut() {
        let mut fixture = Fixture::default();
        fixture.write(format!("{}\n", "x".repeat(LINE_MAX_CHARS + 1000)).as_bytes());
        fixture.write(format!("{}\n", "y".repeat(MAX_LINE_CELLS + 10)).as_bytes());
        let slice = fixture.read(&LogQuery::builder().build());
        assert_eq!(slice.lines(), &[
            format!("{} [... 1000 chars cut]", "x".repeat(LINE_MAX_CHARS)),
            format!(
                "{} [... {} chars cut]",
                "y".repeat(LINE_MAX_CHARS),
                MAX_LINE_CELLS - LINE_MAX_CHARS
            ),
            "y".repeat(10),
        ]);
    }

    /// The newest lines of a log that is one huge line with no newline are
    /// its last pieces, cut and marked, never nothing: here the line's first
    /// 64 KiB end on an erase, which once left a read of it blank.
    #[test]
    fn the_newest_lines_of_one_endless_line_are_its_last_pieces() {
        let mut fixture = Fixture::default();
        let unit = b"\x1b[2K\x1b[1024Gx";
        let units = RAW_LINE_KEEP_BYTES / unit.len() + 2;
        fixture.write(&unit.repeat(units));
        fixture.write(&vec![b'z'; MAX_LINE_CELLS * 2]);

        let slice = fixture.read(&LogQuery::builder().lines(lines(2)).build());

        let piece = format!(
            "{} [... {} chars cut]",
            "z".repeat(LINE_MAX_CHARS),
            MAX_LINE_CELLS - LINE_MAX_CHARS
        );
        assert_eq!(slice.lines(), &[piece, "z".repeat(MAX_COLUMN)]);
        let all = fixture.read(&LogQuery::builder().lines(lines(5)).build());
        assert_eq!(all.lines().len(), 3);
        assert!(all.lines()[0].trim_start().starts_with("xzz"));
    }

    /// A log written as a holder writes one, in a scratch directory that
    /// lives as long as it does.
    fn gated() -> (TempDir, PathBuf, GatedLog) {
        let scratch = TempDir::new().unwrap();
        let path = scratch.path().join("api.log");
        let log = GatedLog::open(path.clone(), 1 << 30);
        (scratch, path, log)
    }

    /// Every read renders the log as one stream that starts over at each
    /// run, from the reset the holder logs there: a run that ends mid-color
    /// or mid-escape hides nothing of the next, its open line is ended, and
    /// what a run draws on the alternate screen reads as one line wherever
    /// a read starts.
    #[test]
    fn reads_render_the_stream_restarted_at_each_run() {
        let (_scratch, path, mut log) = gated();
        log.start_run();
        log.write(b"run1 before\n\x1b[31mopen\x1b]0;cut");
        let run2 = log.start_run();
        log.write(b"run2 hello\n\x1b[?1049h\x1b[Hfull screen\n");
        log.write(b"still on it\n");
        let end = log.end();

        let every = read_log(&path, &LogQuery::builder().build()).unwrap();
        let newest = read_log(&path, &LogQuery::builder().lines(lines(1)).build()).unwrap();
        let since = read_log(&path, &LogQuery::builder().since(Some(run2)).build()).unwrap();
        let matcher = LineMatcher::of_text("on it", PatternKind::Substring).unwrap();
        let scan = first_match(&path, run2, end, &matcher).unwrap();

        assert_eq!(every.lines(), &[
            "run1 before",
            "open",
            "run2 hello",
            FULL_SCREEN_LINE
        ]);
        assert_eq!(newest.lines(), &[FULL_SCREEN_LINE]);
        assert_eq!(since.lines(), &["run2 hello", FULL_SCREEN_LINE]);
        assert_eq!(*scan.matched(), None);
        assert_eq!(
            *last_lines(&path, run2, end, 5, BLOCK_BYTES as u64)
                .unwrap()
                .lines(),
            ["run2 hello", FULL_SCREEN_LINE]
        );
    }

    /// A read from a cursor inside a line and inside a sequence keeps the
    /// state the stream's rendering is in there: the line goes on from the
    /// cursor, and the sequence is finished rather than read as text.
    #[test]
    fn a_read_from_inside_a_sequence_never_reads_it_as_text() {
        let mut fixture = Fixture::default();
        fixture.write(b"first\nsecond \x1b[3");
        let inside = fixture.write(b"1mred\x1b[0m\nthird\n");

        let since = fixture.read(&LogQuery::builder().since(Some(inside)).build());

        assert_eq!(since.lines(), &["red", "third"]);
    }

    /// A read that starts far into a long log, past the look-back, starts
    /// rendering just past a line feed, wherever the look-back bound falls,
    /// so it never starts inside a sequence; one within the look-back of
    /// the oldest byte starts there.
    #[test]
    fn the_look_back_starts_on_a_line() {
        let filler = b"\x1b[1my\x1b[0m\n";
        let count = RENDER_LOOKBACK_BYTES as usize / filler.len() + 10;
        for offset in 0..filler.len() {
            let mut fixture = Fixture::default();
            fixture.write(&b"x\n".repeat(offset));
            fixture.write(&filler.repeat(count));
            let last = fixture.write(b"last\n");
            let span = Span::open(fixture.path()).unwrap();
            let bytes = fs::read(fixture.path()).unwrap();

            let begin = line_start_before(&span, last).unwrap();
            let early = line_start_before(&span, OutputCursor::new(5)).unwrap();

            assert!(begin <= last && last.bytes_since(begin) <= RENDER_LOOKBACK_BYTES);
            let before = usize::try_from(begin.offset()).unwrap() - 1;
            assert_eq!(bytes[before], LINE_FEED, "offset {offset}");
            assert_eq!(early, OutputCursor::new(0));
        }
    }

    /// Bold switched on this many times makes a line of a few characters
    /// span more raw bytes than a scan hands out at once.
    const LONG_RAW_LINE_SEQUENCES: usize = 20_000;

    /// A line drawn across more raw bytes than a scan hands out at once is
    /// matched at where the line began, not where its last piece did.
    #[test]
    fn a_match_on_a_long_line_reports_where_the_line_began() {
        let mut fixture = Fixture::default();
        fixture.write(b"first\n");
        let mut long = b"needle".to_vec();
        long.extend(b"\x1b[1m".repeat(LONG_RAW_LINE_SEQUENCES));
        long.extend(b" end\n");
        assert!(long.len() > LINE_MAX_BYTES);
        let began = fixture.write(&long);
        let end = fixture.log().end();
        let matcher = LineMatcher::of_text("needle", PatternKind::Substring).unwrap();

        let scan = first_match(fixture.path(), OutputCursor::new(0), end, &matcher).unwrap();

        let matched = scan.matched().clone().expect("the long line matches");
        assert_eq!(matched.at(), began);
        assert_eq!(matched.text(), "needle end");
    }

    /// Once the visitor asks to stop it is never called again, not even for
    /// the line a stop left unfinished.
    #[test]
    fn a_stopped_scan_never_visits_again() {
        let mut fixture = Fixture::default();
        let mut partial = b"partial".to_vec();
        partial.extend(b"\x1b[1m".repeat(LONG_RAW_LINE_SEQUENCES));
        fixture.write(&partial);
        fixture.write(b" rest\nmore\nunfinished");
        let end = fixture.log().end();
        let mut visits = 0;

        visit_lines(fixture.path(), OutputCursor::new(0), end, |_| {
            visits += 1;
            ControlFlow::Break(())
        })
        .unwrap();

        assert_eq!(visits, 1);
    }

    /// Frames a long-lived full-screen program draws in the long-screen
    /// test: far more than the look-back covers.
    const SCREEN_FRAMES: usize = 30_000;

    /// A run that stays on the alternate screen far longer than the look-back
    /// leaves one line in the log, so every path reads it alike from
    /// anywhere: no frame is shown, matched or kept.
    #[test]
    fn a_long_alternate_screen_run_reads_alike_from_anywhere() {
        let (_scratch, path, mut log) = gated();
        let start = log.start_run();
        log.write(b"pre\n\x1b[?1049h");
        for frame in 0..SCREEN_FRAMES {
            log.write(format!("\x1b[Hframe {frame} xxxxxxxxxxxx\n").as_bytes());
        }
        let end = log.end();
        let matcher = LineMatcher::of_text("frame", PatternKind::Substring).unwrap();

        let newest = read_log(&path, &LogQuery::builder().lines(lines(3)).build()).unwrap();
        let since = read_log(&path, &LogQuery::builder().since(Some(start)).build()).unwrap();
        let scan = first_match(&path, start, end, &matcher).unwrap();
        let tail = last_lines(&path, start, end, 5, BLOCK_BYTES as u64).unwrap();

        assert!(end.bytes_since(start) < 128, "{end}");
        assert_eq!(newest.lines(), &["pre", FULL_SCREEN_LINE]);
        assert_eq!(since.lines(), newest.lines());
        assert_eq!(*scan.matched(), None);
        assert_eq!(tail.lines(), newest.lines());
    }

    /// Short runs, far more of them than a fixed count of run starts once
    /// kept.
    const SHORT_RUNS: usize = 1000;

    /// A crash loop reads as each run's lines and nothing between them: the
    /// run killed on the alternate screen hides nothing of the thousand runs
    /// after it, and no run start adds a blank line.
    #[test]
    fn a_crash_loop_reads_as_its_runs() {
        let (_scratch, path, mut log) = gated();
        log.start_run();
        log.write(b"\x1b[?1049hkilled on the full screen\n");
        for run in 0..SHORT_RUNS {
            log.start_run();
            log.write(format!("run {run}\n").as_bytes());
        }

        let all = read_log(
            &path,
            &LogQuery::builder()
                .since(Some(OutputCursor::new(0)))
                .lines(lines(2 * SHORT_RUNS))
                .build(),
        )
        .unwrap();

        let expected: Vec<String> = [FULL_SCREEN_LINE.to_string()]
            .into_iter()
            .chain((0..SHORT_RUNS).map(|run| format!("run {run}")))
            .collect();
        assert_eq!(all.lines(), expected.as_slice());
    }

    /// A rendering fault drops only the line it hit on every read, and each
    /// read counts it.
    #[test]
    fn a_rendering_fault_drops_one_line_on_every_read() {
        let mut fixture = Fixture::default();
        fixture.write(b"one\ntwo\nthree\n");
        let end = fixture.log().end();
        let every = LineMatcher::of_text("t", PatternKind::Substring).unwrap();

        render_fault::arm();
        let slice = fixture.read(&LogQuery::builder().build());
        render_fault::arm();
        let scan = first_match(fixture.path(), OutputCursor::new(0), end, &every).unwrap();
        render_fault::arm();
        let tail = last_lines(
            fixture.path(),
            OutputCursor::new(0),
            end,
            5,
            BLOCK_BYTES as u64,
        )
        .unwrap();

        assert_eq!(slice.lines(), &["two", "three"]);
        assert_eq!(slice.unrendered_lines(), 1);
        assert_eq!(
            scan.matched().as_ref().map(|line| line.text().as_str()),
            Some("two")
        );
        assert_eq!(scan.unrendered_lines(), 1);
        assert_eq!(*tail.lines(), ["two", "three"]);
        assert_eq!(tail.unrendered_lines(), 1);
    }

    /// Times the idle poll test reads from the log's end.
    const IDLE_POLLS: usize = 200;
    /// How long those polls may take, far more than reading nothing needs;
    /// rendering the look-back for each took seconds.
    const IDLE_POLL_BOUND: Duration = Duration::from_secs(1);

    /// A read from the log's end, as an idle `--since` poll makes, returns at
    /// once with nothing, rendering none of the output before it.
    #[test]
    fn an_idle_poll_from_the_end_renders_nothing() {
        let mut fixture = Fixture::default();
        let line = format!("\x1b[32m{}\x1b[0m\n", "x".repeat(200));
        for _ in 0..(2 * RENDER_LOOKBACK_BYTES as usize) / line.len() {
            fixture.write(line.as_bytes());
        }
        let end = fixture.log().end();
        let started = Instant::now();

        for _ in 0..IDLE_POLLS {
            let slice = fixture.read(&LogQuery::builder().since(Some(end)).build());
            assert!(slice.lines().is_empty());
            assert_eq!(slice.cursor(), end);
        }

        assert!(
            started.elapsed() < IDLE_POLL_BOUND,
            "{:?} for {IDLE_POLLS} polls",
            started.elapsed()
        );
    }
}
