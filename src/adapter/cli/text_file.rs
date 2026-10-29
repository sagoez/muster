use std::{
    io::{self, Write},
    path::Path,
};

use atomic_write_file::AtomicWriteFile;

/// Writes `text` to `path` so a reader sees the old file or the new one,
/// never a part.
///
/// # Errors
/// Returns the [`io::Error`] of a failed write.
pub fn write_atomically(path: &Path, text: &str) -> io::Result<()> {
    let mut file = AtomicWriteFile::open(path)?;
    file.write_all(text.as_bytes())?;
    file.commit()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;

    /// The file appears whole, replacing what was there.
    #[test]
    fn a_text_file_is_written_whole() {
        let scratch = TempDir::new().unwrap();
        let path = scratch.path().join("muster.yml");

        write_atomically(&path, "old\n").unwrap();
        write_atomically(&path, "version: 1\n").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "version: 1\n");
    }
}
