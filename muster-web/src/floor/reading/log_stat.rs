use std::time::SystemTime;

use getset::CopyGetters;
use typed_builder::TypedBuilder;

/// What one stat of a process's log found, compared with the next to tell
/// whether the process printed anything.
#[derive(Clone, Copy, Debug, PartialEq, Eq, CopyGetters, TypedBuilder)]
#[getset(get_copy = "pub")]
pub(crate) struct LogStat {
    /// Its size in bytes.
    size: u64,
    /// When it was last written, when the filesystem says.
    modified: Option<SystemTime>,
}
