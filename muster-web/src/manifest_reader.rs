use std::path::Path;

/// How the view looks at a project's manifests (`Cargo.toml`,
/// `package.json`, ...) to pick a process's icon. The composition root
/// reads real files; a test hands fixtures.
pub trait ManifestReader: Send + Sync {
    /// Whether `path` is a regular file.
    fn is_file(&self, path: &Path) -> bool;

    /// The text of `path` when it is a regular UTF-8 file of at most
    /// `limit` bytes; none when it is missing, larger or unreadable.
    fn read_text(&self, path: &Path, limit: u64) -> Option<String>;
}
