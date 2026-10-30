use std::io::{self, Write};

use super::followed_output::FollowedOutput;
use crate::domain::value::OutputCursor;

/// Ends a line.
const LINE_FEED: u8 = b'\n';

/// The line `muster logs -f` prints where `skipped` bytes were rotated away
/// before they were read.
pub fn gap_line(skipped: u64) -> String {
    format!("[muster: {skipped} bytes of output were rotated away before they were read]")
}

/// A followed log written byte for byte, with a line naming each gap on a
/// line of its own.
pub struct GapMarked<'a> {
    /// Where the log goes.
    out: &'a mut dyn Write,
    /// Whether the last byte written ended a line, or nothing was written.
    at_line_start: bool,
}

impl<'a> GapMarked<'a> {
    /// Writes into `out`.
    pub fn new(out: &'a mut dyn Write) -> Self {
        Self {
            out,
            at_line_start: true,
        }
    }
}

impl Write for GapMarked<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let written = self.out.write(bytes)?;
        if let Some(last) = bytes[..written].last() {
            self.at_line_start = *last == LINE_FEED;
        }
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.out.flush()
    }
}

impl FollowedOutput for GapMarked<'_> {
    fn gap(&mut self, skipped: u64, _resumed: OutputCursor) -> io::Result<()> {
        if !self.at_line_start {
            self.out.write_all(&[LINE_FEED])?;
        }
        self.out.write_all(gap_line(skipped).as_bytes())?;
        self.out.write_all(&[LINE_FEED])?;
        self.at_line_start = true;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A gap after a line cut short starts its own line, naming the bytes.
    #[test]
    fn a_gap_is_a_line_of_its_own() {
        let mut bytes = Vec::new();
        let mut out = GapMarked::new(&mut bytes);
        out.write_all(b"partial").unwrap();
        out.gap(42, OutputCursor::new(100)).unwrap();
        out.write_all(b"next\n").unwrap();

        assert_eq!(
            String::from_utf8(bytes).unwrap(),
            format!("partial\n{}\nnext\n", gap_line(42))
        );
    }
}
