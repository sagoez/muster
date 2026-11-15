use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use crate::{ManifestReader, ManifestStamp};

/// Manifests held in memory, each a regular file at its path with the text
/// it holds and when it was written, counting every stat and every read.
#[derive(Default)]
pub(crate) struct FakeManifests {
    files: BTreeMap<PathBuf, (String, SystemTime)>,
    stats: AtomicUsize,
    reads: AtomicUsize,
}

impl FakeManifests {
    /// These manifests with a file at `path` holding `text`, written at the
    /// epoch.
    pub(crate) fn with(self, path: &str, text: &str) -> Self {
        self.written_at(path, text, UNIX_EPOCH)
    }

    /// These manifests with a file at `path` holding `text`, written at
    /// `modified`.
    pub(crate) fn written_at(mut self, path: &str, text: &str, modified: SystemTime) -> Self {
        self.files
            .insert(PathBuf::from(path), (text.to_string(), modified));
        self
    }

    /// How many times a manifest was statted.
    pub(crate) fn stats(&self) -> usize {
        self.stats.load(Ordering::SeqCst)
    }

    /// How many times a manifest was read.
    pub(crate) fn reads(&self) -> usize {
        self.reads.load(Ordering::SeqCst)
    }
}

impl ManifestReader for FakeManifests {
    fn stat(&self, path: &Path) -> Option<ManifestStamp> {
        self.stats.fetch_add(1, Ordering::SeqCst);
        let (text, modified) = self.files.get(path)?;
        Some(
            ManifestStamp::builder()
                .modified(Some(*modified))
                .len(u64::try_from(text.len()).ok()?)
                .build(),
        )
    }

    fn read_text(&self, path: &Path, limit: u64) -> Option<String> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        let (text, _) = self.files.get(path)?;
        (u64::try_from(text.len()).ok()? <= limit).then(|| text.clone())
    }
}
