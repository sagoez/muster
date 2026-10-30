use super::{
    error_line::{is_continuation, is_error_line},
    kept::Kept,
    line_filter::LineFilter,
    log_reader::ERROR_CONTEXT_LINES,
};
use crate::domain::value::OutputCursor;

/// A filter applied line by line, with what errors-only needs to remember.
pub(super) struct FilterState<'a> {
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
    pub(super) fn new(filter: &'a LineFilter) -> Self {
        Self {
            filter,
            context: 0,
            previous_kept: false,
        }
    }

    /// Passes `text`, a line that begins at `at`, through the filter into
    /// `kept`.
    pub(super) fn offer(&mut self, text: String, at: OutputCursor, kept: &mut Kept) {
        let keep = match self.filter {
            LineFilter::All => true,
            LineFilter::Grep(matcher) => matcher.is_match(&text),
            LineFilter::ErrorsOnly if is_error_line(&text) => {
                if !self.previous_kept {
                    kept.separate(at);
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
            kept.push(text, at);
        }
        self.previous_kept = keep;
    }
}
