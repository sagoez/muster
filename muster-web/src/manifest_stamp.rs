use std::time::SystemTime;

use getset::CopyGetters;
use typed_builder::TypedBuilder;

/// How a manifest stood when it was statted: when it was last written and
/// how long it was, so a stat that differs says it may read otherwise now.
#[derive(Clone, Copy, Debug, PartialEq, Eq, CopyGetters, TypedBuilder)]
#[getset(get_copy = "pub")]
pub struct ManifestStamp {
    /// When it was last written, none where the platform does not say.
    modified: Option<SystemTime>,
    /// Its length in bytes.
    size: u64,
}
