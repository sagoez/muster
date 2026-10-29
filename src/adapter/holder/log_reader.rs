//! Reading a process's log by output cursor: mapping the cursor through the
//! live and rotated files, finding the newest lines without reading the whole
//! log, stripping and filtering them, and capping what is returned.

use std::{
    collections::VecDeque,
    fs::{self, File},
    io::{self, BufRead, BufReader, Read},
    os::unix::fs::{FileExt, MetadataExt},
    path::Path,
};

use super::{
    error::HolderError,
    error_line::{is_continuation, is_error_line},
    line_filter::LineFilter,
    log::rotated_path,
    log_base::{Generation, LogBase},
    log_query::LogQuery,
    log_slice::LogSlice,
    strip::strip_line,
};
use crate::domain::value::OutputCursor;

/// The longest line a read returns, in characters; the rest is cut.
pub const LINE_MAX_CHARS: usize = 2000;
/// Continuation lines kept after each error line.
pub const ERROR_CONTEXT_LINES: usize = 12;
/// Separates non-adjacent error groups.
pub const GROUP_SEPARATOR: &str = "--";
/// Raw bytes of one line kept for stripping and matching; the rest of a
/// longer line is skipped, so one huge line cannot exhaust memory.
const RAW_LINE_KEEP_BYTES: usize = 64 * 1024;
/// Bytes read per step when scanning back from the end, and buffered when
/// reading forward.
const BLOCK_BYTES: usize = 64 * 1024;
/// Times the files are opened again when a rotation moves them mid-open.
const OPEN_ATTEMPTS: usize = 3;
/// Ends a line.
const LINE_FEED: u8 = b'\n';
/// Opens the note on a cut line, before the count of characters cut.
const CUT_OPEN: &str = " [... ";
/// Closes the note on a cut line.
const CUT_CLOSE: &str = " chars cut]";

/// Reads what `query` asks of the log at `log` and its rotated generation.
/// With a cursor, output after it, starting at the oldest kept byte when
/// rotation discarded what the cursor names; without one, the newest lines
/// that pass the filter. The returned cursor is the end of the log as read.
///
/// # Errors
/// Returns [`HolderError::CursorAhead`] for a cursor past the end of the
/// log, or [`HolderError::Io`] when the files cannot be read.
pub fn read_log(log: &Path, query: &LogQuery) -> Result<LogSlice, HolderError> {
    let span = Span::open(log)?;
    let (start, rotated_away_bytes) = match query.since() {
        Some(since) if since > span.end => {
            return Err(HolderError::CursorAhead {
                cursor: since,
                end: span.end,
            });
        },
        Some(since) => (since.max(span.oldest), span.oldest.bytes_since(since)),
        None => (span.newest_start(query)?, 0),
    };
    let mut kept = Kept::new(query);
    let mut filter = FilterState::new(query.filter());
    let mut lines = BufReader::with_capacity(BLOCK_BYTES, SpanReader {
        span: &span,
        at: start,
    });
    while let Some(line) = next_line(&mut lines)? {
        filter.offer(line, &mut kept);
    }
    Ok(LogSlice::builder()
        .from(start)
        .cursor(span.end)
        .omitted_lines(kept.omitted)
        .lines(kept.into_lines())
        .rotated_away_bytes(rotated_away_bytes)
        .build())
}

/// Every byte still kept of the log at `log` and its rotated generation, the
/// older first, exactly as written: the span [`read_log`] reads, unstripped,
/// for a reader that wants the raw stream.
///
/// # Errors
/// Returns [`HolderError::Io`] when the files cannot be read.
pub fn read_log_bytes(log: &Path) -> Result<Vec<u8>, HolderError> {
    let span = Span::open(log)?;
    let mut bytes = Vec::new();
    SpanReader {
        span: &span,
        at: span.oldest,
    }
    .read_to_end(&mut bytes)?;
    Ok(bytes)
}

/// Reads the next line from `reader`, stripped, keeping at most
/// [`RAW_LINE_KEEP_BYTES`] of it and cutting what strips past
/// [`LINE_MAX_CHARS`]. A final line with no line feed is returned too.
///
/// # Errors
/// Returns an [`io::Error`] when the read fails.
fn next_line(reader: &mut impl BufRead) -> io::Result<Option<String>> {
    let mut head = Vec::new();
    let mut skipped = 0usize;
    let mut read_any = false;
    loop {
        let buffer = reader.fill_buf()?;
        if buffer.is_empty() {
            break;
        }
        read_any = true;
        let (taken, ended) = match buffer.iter().position(|byte| *byte == LINE_FEED) {
            Some(at) => (at, true),
            None => (buffer.len(), false),
        };
        let room = RAW_LINE_KEEP_BYTES.saturating_sub(head.len());
        head.extend_from_slice(&buffer[..taken.min(room)]);
        skipped = skipped.saturating_add(taken.saturating_sub(room));
        reader.consume(taken + usize::from(ended));
        if ended {
            break;
        }
    }
    if !read_any {
        return Ok(None);
    }
    Ok(Some(cut(strip_line(&head), skipped)))
}

/// `text` cut to [`LINE_MAX_CHARS`] with a note of how much went, counting
/// `skipped` raw bytes never stripped as one character each.
fn cut(text: String, skipped: usize) -> String {
    let length = text.chars().count();
    if length <= LINE_MAX_CHARS && skipped == 0 {
        return text;
    }
    let dropped = length
        .saturating_sub(LINE_MAX_CHARS)
        .saturating_add(skipped);
    let kept: String = text.chars().take(LINE_MAX_CHARS).collect();
    format!("{kept}{CUT_OPEN}{dropped}{CUT_CLOSE}")
}

/// The newest lines a read keeps, dropping the oldest past the line or byte
/// cap. Group separators take bytes but do not count as lines.
struct Kept {
    /// The lines, oldest first.
    lines: VecDeque<String>,
    /// Separators among the lines.
    separators: usize,
    /// Bytes the lines take with their line feeds.
    bytes: usize,
    /// Lines dropped to stay within the caps.
    omitted: usize,
    /// The most lines kept.
    line_cap: usize,
    /// The most bytes kept.
    byte_cap: usize,
}

impl Kept {
    /// An empty collection with the caps of `query`.
    fn new(query: &LogQuery) -> Self {
        Self {
            lines: VecDeque::new(),
            separators: 0,
            bytes: 0,
            omitted: 0,
            line_cap: query.lines().into_inner(),
            byte_cap: query.max_bytes().into_inner(),
        }
    }

    /// Adds a line that passed the filter.
    fn push(&mut self, line: String) {
        self.bytes += line.len() + 1;
        self.lines.push_back(line);
        self.enforce_caps();
    }

    /// Adds a separator between error groups, unless nothing precedes it.
    fn separate(&mut self) {
        if !self.lines.is_empty() {
            self.separators += 1;
            self.push(GROUP_SEPARATOR.to_string());
        }
    }

    /// Drops the oldest lines until the caps hold.
    fn enforce_caps(&mut self) {
        while self.lines.len() - self.separators > self.line_cap || self.bytes > self.byte_cap {
            if let Some(line) = self.lines.pop_front() {
                self.bytes -= line.len() + 1;
                self.omitted += 1;
            }
            if self
                .lines
                .front()
                .is_some_and(|line| line == GROUP_SEPARATOR)
            {
                self.lines.pop_front();
                self.separators -= 1;
                self.bytes -= GROUP_SEPARATOR.len() + 1;
            }
        }
    }

    /// The kept lines, oldest first.
    fn into_lines(self) -> Vec<String> {
        self.lines.into()
    }
}

/// A filter applied line by line, with what errors-only needs to remember.
struct FilterState<'a> {
    /// The filter.
    filter: &'a LineFilter,
    /// Continuation lines the current error group may still take.
    context: usize,
    /// Whether the previous line was kept, so adjacent groups need no
    /// separator.
    previous_kept: bool,
}

impl<'a> FilterState<'a> {
    /// A fresh state for `filter`.
    fn new(filter: &'a LineFilter) -> Self {
        Self {
            filter,
            context: 0,
            previous_kept: false,
        }
    }

    /// Passes `text` through the filter into `kept`.
    fn offer(&mut self, text: String, kept: &mut Kept) {
        let keep = match self.filter {
            LineFilter::All => true,
            LineFilter::Grep(matcher) => matcher.is_match(&text),
            LineFilter::ErrorsOnly if is_error_line(&text) => {
                if !self.previous_kept {
                    kept.separate();
                }
                self.context = ERROR_CONTEXT_LINES;
                true
            },
            LineFilter::ErrorsOnly if self.context > 0 && is_continuation(&text) => {
                self.context -= 1;
                true
            },
            LineFilter::ErrorsOnly => {
                self.context = 0;
                false
            },
        };
        if keep {
            kept.push(text);
        }
        self.previous_kept = keep;
    }
}

/// One generation's file and where it sits in the output stream.
struct Piece {
    /// The open file; a rotation renaming it does not disturb the read.
    file: File,
    /// The absolute offset of the first byte read from it.
    start: u64,
    /// Bytes read from it, at most its length when it was opened.
    len: u64,
    /// Bytes at the front of the file that fall before offset zero, which
    /// only a sidecar that disagrees with the files can leave.
    skip: u64,
}

/// The log's kept output as one stream, the rotated generation then the live
/// one, fixed at the lengths they had when opened.
struct Span {
    /// The generations present, oldest first.
    pieces: Vec<Piece>,
    /// The oldest byte still kept.
    oldest: OutputCursor,
    /// Just past the newest byte.
    end: OutputCursor,
}

impl Span {
    /// Opens both generations of `log` and places them in the stream through
    /// the sidecar. When a rotation moved the files mid-open, so the pair
    /// opened is not what the paths name afterwards or the sidecar fits
    /// neither, it opens them again; the last attempt reads what it has.
    ///
    /// # Errors
    /// Returns an [`io::Error`] when a file exists and cannot be opened.
    fn open(log: &Path) -> io::Result<Self> {
        let rotated_log = rotated_path(log);
        let mut attempt = 0;
        loop {
            attempt += 1;
            let rotated = open_generation(&rotated_log)?;
            let live = open_generation(log)?;
            let rotated_generation = rotated.as_ref().map(|(_, generation)| *generation);
            let live_generation = live.as_ref().map(|(_, generation)| *generation);
            let resolved =
                LogBase::resolve(LogBase::read(log), live_generation, rotated_generation);
            let settled = resolved.exact()
                && still_names(&rotated_log, rotated_generation)?
                && still_names(log, live_generation)?;
            if !settled && attempt < OPEN_ATTEMPTS {
                continue;
            }
            let base = resolved.offset();
            let rotated = rotated.map(|(file, generation)| {
                let len = generation.len().min(base);
                Piece {
                    file,
                    start: base - len,
                    len,
                    skip: generation.len() - len,
                }
            });
            let live = live.map(|(file, generation)| Piece {
                file,
                start: base,
                len: generation.len(),
                skip: 0,
            });
            let pieces: Vec<Piece> = rotated.into_iter().chain(live).collect();
            let oldest = pieces.first().map_or(base, |piece| piece.start);
            let end = pieces.last().map_or(base, |piece| piece.start + piece.len);
            return Ok(Self {
                pieces,
                oldest: OutputCursor::new(oldest),
                end: OutputCursor::new(end.max(base)),
            });
        }
    }

    /// Reads into `buffer` from absolute `offset`, from the one piece holding
    /// it; zero at the end of the stream.
    ///
    /// # Errors
    /// Returns an [`io::Error`] when the read fails.
    fn read_at(&self, offset: u64, buffer: &mut [u8]) -> io::Result<usize> {
        let Some(piece) = self
            .pieces
            .iter()
            .find(|piece| offset >= piece.start && offset < piece.start + piece.len)
        else {
            return Ok(0);
        };
        let available = piece.start + piece.len - offset;
        let wanted = buffer
            .len()
            .min(usize::try_from(available).unwrap_or(usize::MAX));
        piece
            .file
            .read_at(&mut buffer[..wanted], offset - piece.start + piece.skip)
    }

    /// Fills `buffer` from absolute `offset`, across pieces; returns the bytes
    /// read, fewer only at the end of the stream or of a file cut short.
    ///
    /// # Errors
    /// Returns an [`io::Error`] when a read fails.
    fn read_full_at(&self, offset: u64, buffer: &mut [u8]) -> io::Result<usize> {
        let mut filled = 0;
        while filled < buffer.len() {
            let read = self.read_at(offset + filled as u64, &mut buffer[filled..])?;
            if read == 0 {
                break;
            }
            filled += read;
        }
        Ok(filled)
    }

    /// Where reading must start for the newest `query.lines()` lines that
    /// pass the filter: scans back from the end in blocks, testing each line
    /// once, and stops at the line that completes the count. Errors-only
    /// counts error lines, each of which opens a group of at least one line.
    ///
    /// # Errors
    /// Returns an [`io::Error`] when a read fails.
    fn newest_start(&self, query: &LogQuery) -> io::Result<OutputCursor> {
        let wanted = query.lines().into_inner();
        let oldest = self.oldest.offset();
        let end = self.end.offset();
        let mut counted = 0;
        let mut block = vec![0; BLOCK_BYTES];
        let mut high = end;
        let mut line_end = end;
        while high > oldest {
            let low = high.saturating_sub(BLOCK_BYTES as u64).max(oldest);
            let length = (high - low) as usize;
            let filled = self.read_full_at(low, &mut block[..length])?;
            for index in (0..filled).rev() {
                if block[index] != LINE_FEED {
                    continue;
                }
                let line_start = low + index as u64 + 1;
                let trailing = line_start == end;
                if !trailing && self.counts(query, line_start, line_end, (low, &block[..filled]))? {
                    counted += 1;
                    if counted >= wanted {
                        return Ok(OutputCursor::new(line_start));
                    }
                }
                line_end = line_start - 1;
            }
            high = low;
        }
        Ok(self.oldest)
    }

    /// Whether the line `[start, end)` counts toward the newest lines wanted,
    /// reading its head from `block` (which starts at the given offset) when
    /// it lies there, else from the files.
    ///
    /// # Errors
    /// Returns an [`io::Error`] when a read fails.
    fn counts(
        &self,
        query: &LogQuery,
        start: u64,
        end: u64,
        block: (u64, &[u8]),
    ) -> io::Result<bool> {
        if matches!(query.filter(), LineFilter::All) {
            return Ok(true);
        }
        let head_end = end.min(start + RAW_LINE_KEEP_BYTES as u64);
        let (block_start, bytes) = block;
        let text = if head_end <= block_start + bytes.len() as u64 {
            strip_line(&bytes[(start - block_start) as usize..(head_end - block_start) as usize])
        } else {
            let mut head = vec![0; (head_end - start) as usize];
            let filled = self.read_full_at(start, &mut head)?;
            strip_line(&head[..filled])
        };
        Ok(match query.filter() {
            LineFilter::All => true,
            LineFilter::Grep(matcher) => matcher.is_match(&text),
            LineFilter::ErrorsOnly => is_error_line(&text),
        })
    }
}

/// `path` opened with its identity and length, or none when it is absent.
///
/// # Errors
/// Returns an [`io::Error`] when it exists and cannot be opened.
fn open_generation(path: &Path) -> io::Result<Option<(File, Generation)>> {
    match File::open(path) {
        Ok(file) => {
            let generation = Generation::of(&file)?;
            Ok(Some((file, generation)))
        },
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

/// Whether `path` still names the file `opened` was taken from, or still
/// names nothing when nothing was opened.
///
/// # Errors
/// Returns an [`io::Error`] when the path exists and cannot be inspected.
fn still_names(path: &Path, opened: Option<Generation>) -> io::Result<bool> {
    let now = match fs::metadata(path) {
        Ok(metadata) => Some(metadata.ino()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    Ok(now == opened.map(|generation| generation.inode()))
}

/// A forward reader over a [`Span`] from an absolute offset.
struct SpanReader<'a> {
    /// The stream read.
    span: &'a Span,
    /// The next offset to read.
    at: OutputCursor,
}

impl Read for SpanReader<'_> {
    /// Reads from the current offset onward.
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let read = self.span.read_at(self.at.offset(), buffer)?;
        self.at = self.at.advanced(read as u64);
        Ok(read)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use bytesize::ByteSize as RawByteSize;
    use tempfile::TempDir;

    use super::*;
    use crate::{
        adapter::holder::{
            byte_limit::ByteLimit, line_limit::LineLimit, line_matcher::LineMatcher,
            log::RotatingLog,
        },
        domain::{
            process::{LogPattern, PatternKind, PatternText},
            value::ByteSize,
        },
    };

    /// A log in a scratch directory, written through the holder's own log.
    struct Fixture {
        _scratch: TempDir,
        path: PathBuf,
        log: RotatingLog,
    }

    impl Fixture {
        /// A fresh log rotating at `cap` bytes.
        fn with_cap(cap: u64) -> Self {
            let scratch = TempDir::new().unwrap();
            let path = scratch.path().join("api.log");
            let log = RotatingLog::open(
                path.clone(),
                ByteSize::try_new(RawByteSize::b(cap)).unwrap(),
            )
            .unwrap();
            Self {
                _scratch: scratch,
                path,
                log,
            }
        }

        /// A log that never rotates in these tests.
        fn new() -> Self {
            Self::with_cap(1 << 30)
        }

        fn write(&mut self, bytes: &[u8]) -> OutputCursor {
            let at = self.log.end();
            self.log.append(bytes).unwrap();
            at
        }

        fn read(&self, query: &LogQuery) -> LogSlice {
            read_log(&self.path, query).unwrap()
        }
    }

    fn lines(count: usize) -> LineLimit {
        LineLimit::try_new(count).unwrap()
    }

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

    fn numbered(range: std::ops::Range<usize>) -> String {
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
        let mut fixture = Fixture::new();
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

    /// Output a prompt left without a line feed is read as a line.
    #[test]
    fn a_trailing_partial_line_is_returned() {
        let mut fixture = Fixture::new();
        fixture.write(b"first\nName? ");
        let slice = fixture.read(&LogQuery::builder().build());
        assert_eq!(slice.lines(), &["first", "Name?"]);
    }

    #[test]
    fn escapes_are_stripped_and_progress_collapses() {
        let mut fixture = Fixture::new();
        fixture.write(b"\x1b[32mready\x1b[0m\r\n10%\r50%\r100%\r\n");
        let slice = fixture.read(&LogQuery::builder().build());
        assert_eq!(slice.lines(), &["ready", "100%"]);
    }

    /// A cursor keeps working across rotations; one older than the oldest
    /// kept byte reads from there and says how much rotation discarded.
    #[test]
    fn cursors_map_across_rotation() {
        let mut fixture = Fixture::with_cap(12);
        let first = fixture.write(b"aaaa\nbbbb\n");
        let second = fixture.write(b"cccc\ndddd\n");
        fixture.write(b"eeee\n");
        assert!(fs::metadata(rotated_path(&fixture.path)).is_ok());

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
        assert!(read_log_bytes(&fixture.path).unwrap().is_empty());
        fixture.write(b"\x1b[31maaaa\n");
        fixture.write(b"live \xff\n");

        assert_eq!(
            read_log_bytes(&fixture.path).unwrap(),
            b"\x1b[31maaaa\nlive \xff\n"
        );
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
        let mut fixture = Fixture::new();
        fixture.write(b"one\n");
        let error = read_log(
            &fixture.path,
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
        let mut fixture = Fixture::new();
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
        let mut fixture = Fixture::new();
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
        let mut fixture = Fixture::new();
        let start = fixture.write(numbered(0..10).as_bytes());
        let by_lines = fixture.read(
            &LogQuery::builder()
                .since(Some(start))
                .lines(lines(4))
                .build(),
        );
        assert_eq!(by_lines.lines(), &["line 6", "line 7", "line 8", "line 9"]);
        assert_eq!(by_lines.omitted_lines(), 6);
        assert_eq!(by_lines.from(), start);

        let by_bytes = fixture.read(
            &LogQuery::builder()
                .since(Some(start))
                .max_bytes(ByteLimit::try_new(14).unwrap())
                .build(),
        );
        assert_eq!(by_bytes.lines(), &["line 8", "line 9"]);
        assert_eq!(by_bytes.omitted_lines(), 8);

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

    /// A line past the character limit is cut with a note, and one past the
    /// raw limit is never held whole.
    #[test]
    fn long_lines_are_cut() {
        let mut fixture = Fixture::new();
        fixture.write(format!("{}\n", "x".repeat(LINE_MAX_CHARS + 1000)).as_bytes());
        fixture.write(format!("{}\n", "y".repeat(RAW_LINE_KEEP_BYTES + 10)).as_bytes());
        let slice = fixture.read(&LogQuery::builder().build());
        assert_eq!(
            slice.lines()[0],
            format!("{} [... 1000 chars cut]", "x".repeat(LINE_MAX_CHARS))
        );
        assert_eq!(
            slice.lines()[1],
            format!(
                "{} [... {} chars cut]",
                "y".repeat(LINE_MAX_CHARS),
                RAW_LINE_KEEP_BYTES + 10 - LINE_MAX_CHARS
            )
        );
    }
}
