use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};

use crate::ManifestReader;

/// Manifests held in memory, each a regular file at its path, counting
/// every look at one.
#[derive(Default)]
pub(crate) struct FakeManifests {
    files: BTreeMap<PathBuf, String>,
    looks: AtomicUsize,
}

impl FakeManifests {
    /// These manifests with a file at `path` holding `text`.
    pub(crate) fn with(mut self, path: &str, text: &str) -> Self {
        self.files.insert(PathBuf::from(path), text.to_string());
        self
    }

    /// How many times a manifest was looked for or read.
    pub(crate) fn looks(&self) -> usize {
        self.looks.load(Ordering::SeqCst)
    }
}

impl ManifestReader for FakeManifests {
    fn is_file(&self, path: &Path) -> bool {
        self.looks.fetch_add(1, Ordering::SeqCst);
        self.files.contains_key(path)
    }

    fn read_text(&self, path: &Path, limit: u64) -> Option<String> {
        self.looks.fetch_add(1, Ordering::SeqCst);
        let text = self.files.get(path)?;
        (u64::try_from(text.len()).ok()? <= limit).then(|| text.clone())
    }
}
