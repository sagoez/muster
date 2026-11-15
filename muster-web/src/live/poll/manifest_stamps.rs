use std::path::{Path, PathBuf};

use typed_builder::TypedBuilder;

use crate::{ManifestReader, ManifestStamp};

/// The stamp each manifest path stood at when a resolution first looked at
/// it, none for one that was not a regular file, in the order looked. Its
/// builder takes nothing, so every set starts empty. It has no getters: it
/// is an owned cache, read only through its methods.
#[derive(Debug, Default, TypedBuilder)]
pub(super) struct ManifestStamps {
    /// Each path stamped, with its stamp.
    #[builder(default, setter(skip))]
    stamps: Vec<(PathBuf, Option<ManifestStamp>)>,
}

impl ManifestStamps {
    /// Whether `path` was stamped yet.
    pub(super) fn holds(&self, path: &Path) -> bool {
        self.stamps.iter().any(|(kept, _)| kept == path)
    }

    /// Keeps `stamp` for `path` unless it was already stamped.
    pub(super) fn keep(&mut self, path: &Path, stamp: Option<ManifestStamp>) {
        if !self.holds(path) {
            self.stamps.push((path.to_path_buf(), stamp));
        }
    }

    /// Whether every path stamped still stands as stamped, by a stat of each
    /// over `manifests`.
    pub(super) fn still_stand(&self, manifests: &dyn ManifestReader) -> bool {
        self.stamps
            .iter()
            .all(|(path, stamp)| manifests.stat(path) == *stamp)
    }
}
