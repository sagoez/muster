use std::time::SystemTime;

use typed_builder::TypedBuilder;

/// How a manifest stood when it was statted: when it was last written and
/// how long it was, so a stat that differs says it may read otherwise now.
/// It has no getters: it is only ever compared whole.
#[derive(Clone, Copy, Debug, PartialEq, Eq, TypedBuilder)]
pub struct ManifestStamp {
    /// When it was last written, none where the platform does not say.
    modified: Option<SystemTime>,
    /// Its length in bytes.
    len: u64,
}
