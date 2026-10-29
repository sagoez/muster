use getset::{CopyGetters, Getters};
use typed_builder::TypedBuilder;

use super::{byte_limit::ByteLimit, line_filter::LineFilter, line_limit::LineLimit};
use crate::domain::value::OutputCursor;

/// What to read of a process's log.
#[derive(Debug, Clone, Getters, CopyGetters, TypedBuilder)]
pub struct LogQuery {
    /// Read only output after this cursor; without it, read the newest lines.
    #[getset(get_copy = "pub")]
    #[builder(default)]
    since: Option<OutputCursor>,
    /// The most lines returned; the oldest are dropped past it.
    #[getset(get_copy = "pub")]
    #[builder(default)]
    lines: LineLimit,
    /// Which lines are kept.
    #[getset(get = "pub")]
    #[builder(default)]
    filter: LineFilter,
    /// The most bytes of text returned; the oldest lines are dropped past it.
    #[getset(get_copy = "pub")]
    #[builder(default)]
    max_bytes: ByteLimit,
}
