use getset::Getters;
use typed_builder::TypedBuilder;

use crate::domain::{process::LogPattern, value::OutputCursor};

/// What an adopting handle asks its held-back reader to read back from the
/// log before the holder's frames: the lines from `from` that one of
/// `patterns` matches.
#[derive(Debug, Clone, Getters, TypedBuilder)]
#[getset(get = "pub(super)")]
pub(super) struct ReplayRequest {
    /// Where the read starts.
    from: OutputCursor,
    /// What a line must match to be forwarded.
    patterns: Vec<LogPattern>,
}
