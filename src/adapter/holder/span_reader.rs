use std::io::{self, Read};

use super::span::Span;
use crate::domain::value::OutputCursor;

/// A forward reader over a [`Span`] from an absolute offset.
pub(super) struct SpanReader<'a> {
    /// The stream read.
    span: &'a Span,
    /// The next offset to read.
    at: OutputCursor,
}

impl<'a> SpanReader<'a> {
    /// A reader of `span` starting at `at`.
    pub(super) fn new(span: &'a Span, at: OutputCursor) -> Self {
        Self { span, at }
    }
}

impl Read for SpanReader<'_> {
    /// Reads from the current offset onward.
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let read = self.span.read_at(self.at.offset(), buffer)?;
        self.at = self.at.advanced(read as u64);
        Ok(read)
    }
}
