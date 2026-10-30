use std::{
    fs::{self, File},
    io,
    os::unix::fs::{FileExt, MetadataExt},
    path::Path,
};

use getset::CopyGetters;

use super::{
    error_line::is_error_line,
    generation::Generation,
    line_filter::LineFilter,
    log::rotated_path,
    log_base::LogBase,
    log_query::LogQuery,
    log_reader::{BLOCK_BYTES, LINE_FEED, RAW_LINE_KEEP_BYTES},
    piece::Piece,
    strip::strip_line,
};
use crate::domain::value::OutputCursor;

/// Times the files are opened again when a rotation moves them mid-open.
const OPEN_ATTEMPTS: usize = 3;

/// The log's kept output as one stream, the rotated generation then the live
/// one, fixed at the lengths they had when opened.
#[derive(CopyGetters)]
pub(super) struct Span {
    /// The generations present, oldest first.
    pieces: Vec<Piece>,
    /// The oldest byte still kept.
    #[getset(get_copy = "pub(super)")]
    oldest: OutputCursor,
    /// Just past the newest byte.
    #[getset(get_copy = "pub(super)")]
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
    pub(super) fn open(log: &Path) -> io::Result<Self> {
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
                Piece::builder()
                    .file(file)
                    .start(base - len)
                    .len(len)
                    .skip(generation.len() - len)
                    .build()
            });
            let live = live.map(|(file, generation)| {
                Piece::builder()
                    .file(file)
                    .start(base)
                    .len(generation.len())
                    .skip(0)
                    .build()
            });
            let pieces: Vec<Piece> = rotated.into_iter().chain(live).collect();
            let oldest = pieces.first().map_or(base, Piece::start);
            let end = pieces.last().map_or(base, Piece::end);
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
    pub(super) fn read_at(&self, offset: u64, buffer: &mut [u8]) -> io::Result<usize> {
        let Some(piece) = self.pieces.iter().find(|piece| piece.holds(offset)) else {
            return Ok(0);
        };
        let available = piece.end() - offset;
        let wanted = buffer
            .len()
            .min(usize::try_from(available).unwrap_or(usize::MAX));
        piece
            .file()
            .read_at(&mut buffer[..wanted], offset - piece.start() + piece.skip())
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
    pub(super) fn newest_start(&self, query: &LogQuery) -> io::Result<OutputCursor> {
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
    /// it lies there, else from the files. A line that only redrew the
    /// screen never counts, as a read leaves it out.
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
        let head_end = end.min(start + RAW_LINE_KEEP_BYTES as u64);
        let (block_start, bytes) = block;
        let text = if head_end <= block_start + bytes.len() as u64 {
            strip_line(&bytes[(start - block_start) as usize..(head_end - block_start) as usize])
        } else {
            let mut head = vec![0; (head_end - start) as usize];
            let filled = self.read_full_at(start, &mut head)?;
            strip_line(&head[..filled])
        };
        let Some(text) = text else {
            return Ok(false);
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
