use getset::CopyGetters;
use typed_builder::TypedBuilder;

/// A live file's start offset, and whether the sidecar pinned it exactly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, CopyGetters, TypedBuilder)]
#[getset(get_copy = "pub(super)")]
pub(super) struct Resolved {
    /// The absolute offset of the live file's first byte.
    offset: u64,
    /// Whether the sidecar belongs to the files found; otherwise the offset
    /// is the sidecar's own, the best available.
    exact: bool,
}
