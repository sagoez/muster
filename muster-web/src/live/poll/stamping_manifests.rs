use std::{
    path::Path,
    sync::{Mutex, PoisonError},
};

use muster_os::locked::Locked;
use typed_builder::TypedBuilder;

use super::manifest_stamps::ManifestStamps;
use crate::{ManifestReader, ManifestStamp};

/// A reader that passes every look on to `inner` and stamps each path the
/// first time it is looked at, before any read of it, so a manifest that
/// changes mid-resolution stamps as it was and reads as changed next time.
/// It has no getters: it is an owned cache, its stamps taken whole by
/// [`StampingManifests::into_stamps`].
#[derive(TypedBuilder)]
pub(super) struct StampingManifests<'a> {
    /// The reader every look goes to.
    inner: &'a dyn ManifestReader,
    /// Each path looked at so far, with its stamp then, in the order looked.
    #[builder(default, setter(skip))]
    stamps: Mutex<ManifestStamps>,
}

impl StampingManifests<'_> {
    /// Every path looked at, each with the stamp it first had.
    pub(super) fn into_stamps(self) -> ManifestStamps {
        self.stamps
            .into_inner()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Keeps `stamp` for `path` unless it was already stamped.
    fn keep(&self, path: &Path, stamp: Option<ManifestStamp>) {
        self.stamps.locked().keep(path, stamp);
    }

    /// Whether `path` was stamped yet.
    fn stamped(&self, path: &Path) -> bool {
        self.stamps.locked().holds(path)
    }
}

impl ManifestReader for StampingManifests<'_> {
    fn stat(&self, path: &Path) -> Option<ManifestStamp> {
        let stamp = self.inner.stat(path);
        self.keep(path, stamp);
        stamp
    }

    fn read_text(&self, path: &Path, limit: u64) -> Option<String> {
        if !self.stamped(path) {
            self.keep(path, self.inner.stat(path));
        }
        self.inner.read_text(path, limit)
    }
}
