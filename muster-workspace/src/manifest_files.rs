//! The manifests `muster view` picks icons from, read from the real files.

use std::{fs, path::Path};

use muster_os::bounded_read::read_bounded;
use muster_web::ManifestReader;

/// Reads manifests from disk: a regular file only, never waited on, and
/// never more than the caller's limit.
#[derive(Clone, Copy, Debug, Default)]
pub struct ManifestFiles;

impl ManifestReader for ManifestFiles {
    fn is_file(&self, path: &Path) -> bool {
        fs::metadata(path).is_ok_and(|metadata| metadata.is_file())
    }

    fn read_text(&self, path: &Path, limit: u64) -> Option<String> {
        read_bounded(path, limit).ok()
    }
}

#[cfg(test)]
mod tests {
    use muster_os::test_fifo::make_fifo;
    use tempfile::TempDir;

    use super::*;

    /// The limit the reads under test take.
    const LIMIT: u64 = 16;

    /// A file up to the limit reads whole; a larger, missing or non-UTF-8
    /// file, a folder or a FIFO reads as none, and only a regular file is
    /// a file.
    #[test]
    fn reads_up_to_its_limit_and_a_missing_file_as_none() {
        let scratch = TempDir::new().unwrap();
        let at_limit = scratch.path().join("Cargo.toml");
        fs::write(&at_limit, "a".repeat(16)).unwrap();
        let over = scratch.path().join("package.json");
        fs::write(&over, "a".repeat(17)).unwrap();
        let binary = scratch.path().join("go.mod");
        fs::write(&binary, [0xFF, 0xFE]).unwrap();
        let fifo = scratch.path().join("compose.yaml");
        make_fifo(&fifo);
        let missing = scratch.path().join("pyproject.toml");
        let files = ManifestFiles;

        assert_eq!(
            files.read_text(&at_limit, LIMIT).as_deref(),
            Some("aaaaaaaaaaaaaaaa")
        );
        for unread in [
            &over,
            &binary,
            &fifo,
            &missing,
            &scratch.path().to_path_buf(),
        ] {
            assert_eq!(files.read_text(unread, LIMIT), None, "{}", unread.display());
        }
        assert!(files.is_file(&at_limit));
        assert!(!files.is_file(&missing));
        assert!(!files.is_file(scratch.path()));
        assert!(!files.is_file(&fifo));
    }
}
