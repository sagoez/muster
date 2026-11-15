use std::{
    path::{Path, PathBuf},
    sync::{Mutex, PoisonError},
};

use typed_builder::TypedBuilder;

use crate::{ManifestReader, ManifestStamp};

/// The stamp each manifest path stood at when a resolution first looked
/// at it, none for one that was not a regular file.
pub(super) type ManifestStamps = Vec<(PathBuf, Option<ManifestStamp>)>;

/// A reader that passes every look on to `inner` and stamps each path the
/// first time it is looked at, before any read of it, so a manifest that
/// changes mid-resolution stamps as it was and reads as changed next time.
/// It has no getters: what it stamped is taken whole by
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
        let mut stamps = self.stamps.lock().unwrap_or_else(PoisonError::into_inner);
        if !stamps.iter().any(|(kept, _)| kept == path) {
            stamps.push((path.to_path_buf(), stamp));
        }
    }

    /// Whether `path` was stamped yet.
    fn stamped(&self, path: &Path) -> bool {
        self.stamps
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .any(|(kept, _)| kept == path)
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

/// Whether every path in `stamps` still stands as stamped, by a stat of
/// each over `manifests`.
pub(super) fn still_stamped(stamps: &ManifestStamps, manifests: &dyn ManifestReader) -> bool {
    stamps
        .iter()
        .all(|(path, stamp)| manifests.stat(path) == *stamp)
}
