use std::path::Path;

use crate::ManifestStamp;

/// How the view looks at a project's manifests (`Cargo.toml`,
/// `package.json`, ...) to pick a process's icon. The composition root
/// reads real files; a test hands fixtures.
pub trait ManifestReader: Send + Sync {
    /// The stamp of `path` when it is a regular file; none when it is
    /// missing or anything else. A stat, never a read.
    fn stat(&self, path: &Path) -> Option<ManifestStamp>;

    /// The text of `path` when it is a regular UTF-8 file of at most
    /// `limit` bytes; none when it is missing, larger or unreadable.
    fn read_text(&self, path: &Path, limit: u64) -> Option<String>;
}
