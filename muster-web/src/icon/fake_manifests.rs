use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use crate::ManifestReader;

/// Manifests held in memory, each a regular file at its path.
#[derive(Default)]
pub(super) struct FakeManifests {
    files: BTreeMap<PathBuf, String>,
}

impl FakeManifests {
    /// These manifests with a file at `path` holding `text`.
    pub(super) fn with(mut self, path: &str, text: &str) -> Self {
        self.files.insert(PathBuf::from(path), text.to_string());
        self
    }
}

impl ManifestReader for FakeManifests {
    fn is_file(&self, path: &Path) -> bool {
        self.files.contains_key(path)
    }

    fn read_text(&self, path: &Path, limit: u64) -> Option<String> {
        let text = self.files.get(path)?;
        (u64::try_from(text.len()).ok()? <= limit).then(|| text.clone())
    }
}
