//! A VT parser that never holds an unbounded string payload: vte buffers an
//! OSC payload in full until its terminator, so a child that opens one and
//! never ends it would grow every parser on its stream without limit.

use alacritty_terminal::vte::{Parser, Perform};

use super::{
    escape::{BEL, CAN, ESC, SUB},
    fed::Fed,
    watched_performer::WatchedPerformer,
};

/// Most bytes a parser takes in a row with nothing dispatched before the
/// sequence they belong to is dropped. Only a string (OSC, DCS, APC, PM, SOS)
/// or a malformed sequence runs this long, and a title or hyperlink is far
/// shorter.
pub const MAX_PENDING_BYTES: usize = 64 * 1024;
/// Most bytes [`BoundedParser::advance`] hands the parser at once.
const SLICE: usize = 4 * 1024;
/// The high bits that mark a UTF-8 continuation byte.
const UTF8_CONTINUATION_MASK: u8 = 0b1100_0000;
/// The value of those bits on a continuation byte.
const UTF8_CONTINUATION: u8 = 0b1000_0000;

/// A vte [`Parser`] whose pending sequence is bounded: past
/// [`MAX_PENDING_BYTES`] bytes with nothing dispatched, the parser is reset
/// and the rest of the sequence is dropped up to its terminator (BEL, CAN,
/// SUB, or the ESC that starts the next sequence), where parsing resumes.
#[derive(Default)]
pub struct BoundedParser {
    /// The parser, kept across bytes.
    parser: Parser,
    /// Bytes fed since the parser last dispatched anything.
    pending: usize,
    /// Whether the bytes until the next terminator are being dropped.
    dropping: bool,
}

impl BoundedParser {
    /// A parser at the start of a stream.
    pub fn new() -> Self {
        Self::default()
    }

    /// Feeds `bytes` to `performer` in slices of a few KiB cut
    /// between characters, so the parser parses them as it parses one
    /// chunk. It then holds at most [`MAX_PENDING_BYTES`] plus one slice.
    pub fn advance<P: Perform>(&mut self, performer: &mut P, mut bytes: &[u8]) {
        while !bytes.is_empty() {
            if self.dropping {
                let Some(at) = bytes.iter().position(|&byte| ends_a_sequence(byte)) else {
                    return;
                };
                self.dropping = false;
                bytes = &bytes[if bytes[at] == ESC { at } else { at + 1 }..];
                continue;
            }
            let (slice, rest) = bytes.split_at(slice_end(bytes));
            self.feed(performer, slice);
            bytes = rest;
        }
    }

    /// Feeds one byte to `performer` and says whether it was kept, so a
    /// caller feeding a second parser the same stream can drop it there too.
    pub fn advance_byte<P: Perform>(&mut self, performer: &mut P, byte: u8) -> Fed {
        if self.dropping {
            if byte != ESC {
                self.dropping = !ends_a_sequence(byte);
                return Fed::Dropped;
            }
            self.dropping = false;
        }
        if self.feed(performer, &[byte]) {
            Fed::Kept
        } else {
            Fed::Dropped
        }
    }

    /// Bytes fed since the parser last dispatched anything, which bounds the
    /// payload it holds.
    pub fn pending(&self) -> usize {
        self.pending
    }

    /// Parses `slice` into `performer`, then drops the sequence in progress
    /// once it has run past [`MAX_PENDING_BYTES`]. Returns whether the
    /// parser kept the slice.
    fn feed<P: Perform>(&mut self, performer: &mut P, slice: &[u8]) -> bool {
        let mut watched = WatchedPerformer::new(performer);
        self.parser.advance(&mut watched, slice);
        self.pending = if watched.dispatched() {
            0
        } else {
            self.pending + slice.len()
        };
        if self.pending <= MAX_PENDING_BYTES {
            return true;
        }
        self.parser = Parser::new();
        self.pending = 0;
        self.dropping = true;
        false
    }
}

/// Whether `byte` ends any sequence in progress: BEL, CAN, SUB, or the ESC
/// that starts the next one.
fn ends_a_sequence(byte: u8) -> bool {
    matches!(byte, BEL | CAN | SUB | ESC)
}

/// Where the first slice of `bytes` ends: after [`SLICE`] bytes, moved on
/// past any UTF-8 continuation bytes so no character is split.
fn slice_end(bytes: &[u8]) -> usize {
    let mut end = bytes.len().min(SLICE);
    while end < bytes.len() && is_continuation(bytes[end]) {
        end += 1;
    }
    end
}

/// Whether `byte` continues a UTF-8 character rather than starting one.
fn is_continuation(byte: u8) -> bool {
    byte & UTF8_CONTINUATION_MASK == UTF8_CONTINUATION
}

#[cfg(test)]
mod tests {
    use super::{super::dispatch_recorder::DispatchRecorder, *};

    /// Bytes in the unterminated OSC each test feeds, many times the bound.
    const FLOOD: usize = 4 * 1024 * 1024;

    /// An unterminated multi-megabyte OSC never holds more than the bound,
    /// is dropped whole, and the stream parses again after its terminator.
    #[test]
    fn an_unterminated_osc_stays_bounded_and_the_stream_resyncs() {
        let mut parser = BoundedParser::new();
        let mut recorder = DispatchRecorder::default();
        parser.advance(&mut recorder, b"before\x1b]0;");
        let mut dropped = 0;

        for _ in 0..FLOOD {
            if parser.advance_byte(&mut recorder, b'x') == Fed::Dropped {
                dropped += 1;
            }
            assert!(parser.pending() <= MAX_PENDING_BYTES);
        }
        parser.advance(&mut recorder, b"\x07after\x1b]2;title\x07");

        assert!(dropped >= FLOOD - MAX_PENDING_BYTES);
        assert_eq!(recorder.printed(), "beforeafter");
        assert_eq!(recorder.oscs(), vec![b"2title".to_vec()]);
    }

    /// The same flood in one write, as a log copy hands it over, is dropped
    /// in slices and never held past the bound.
    #[test]
    fn a_flood_in_one_write_is_dropped_too() {
        let mut parser = BoundedParser::new();
        let mut recorder = DispatchRecorder::default();
        let mut stream = b"before\x1b]0;".to_vec();
        stream.extend(vec![b'x'; FLOOD]);
        stream.extend(b"\x07after");

        parser.advance(&mut recorder, &stream);

        assert!(parser.pending() <= MAX_PENDING_BYTES);
        assert_eq!(recorder.printed(), "beforeafter");
        assert!(recorder.oscs().is_empty());
    }

    /// A character is never split between slices, so a chunk parses as vte
    /// parses it whole.
    #[test]
    fn slices_end_between_characters() {
        let mut text = vec![b'a'; SLICE - 1];
        text.extend("é".as_bytes());

        assert_eq!(slice_end(&text), SLICE + 1);
        assert_eq!(slice_end(b"short"), 5);
    }

    /// An escape ends the dropped run and starts the next sequence.
    #[test]
    fn an_escape_ends_the_dropped_run() {
        let mut parser = BoundedParser::new();
        let mut recorder = DispatchRecorder::default();
        parser.advance(&mut recorder, b"\x1b]0;");
        parser.advance(&mut recorder, &vec![b'x'; MAX_PENDING_BYTES + 1]);

        parser.advance(&mut recorder, b"\x1b]2;ok\x07text");

        assert_eq!(recorder.oscs(), vec![b"2ok".to_vec()]);
        assert_eq!(recorder.printed(), "text");
    }

    /// A string within the bound is parsed as vte parses it.
    #[test]
    fn a_string_within_the_bound_is_kept() {
        let mut parser = BoundedParser::new();
        let mut recorder = DispatchRecorder::default();
        let mut sequence = b"\x1b]2;".to_vec();
        sequence.extend(vec![b'y'; MAX_PENDING_BYTES - 8]);
        sequence.push(BEL);

        parser.advance(&mut recorder, &sequence);

        assert_eq!(recorder.oscs().len(), 1);
        assert_eq!(parser.pending(), 0);
    }
}
