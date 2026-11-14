use std::{
    io::{self, Write},
    os::unix::fs::OpenOptionsExt as _,
    path::Path,
};

use atomic_write_file::{AtomicWriteFile, OpenOptions, unix::OpenOptionsExt as _};

/// Mode of a file holding secrets: its owner's alone.
const PRIVATE_FILE_MODE: u32 = 0o600;

/// Writes `text` to `path` so a reader sees the old file or the new one,
/// never a part.
///
/// # Errors
/// Returns the [`io::Error`] of a failed write.
pub fn write_atomically(path: &Path, text: &str) -> io::Result<()> {
    commit(AtomicWriteFile::open(path)?, text)
}

/// Writes `text` to `path` as [`write_atomically`] does, readable by its
/// owner alone whatever mode a file it replaces had.
///
/// # Errors
/// Returns the [`io::Error`] of a failed write.
pub fn write_privately(path: &Path, text: &str) -> io::Result<()> {
    let file = OpenOptions::new()
        .preserve_mode(false)
        .mode(PRIVATE_FILE_MODE)
        .open(path)?;
    commit(file, text)
}

/// Writes `text` into `file` and commits it.
///
/// # Errors
/// Returns the [`io::Error`] of a failed write.
fn commit(mut file: AtomicWriteFile, text: &str) -> io::Result<()> {
    file.write_all(text.as_bytes())?;
    file.commit()
}

#[cfg(test)]
mod tests {
    use std::{
        fs::{self, Permissions},
        os::unix::fs::PermissionsExt,
    };

    use tempfile::TempDir;

    use super::*;

    /// Mode bits a permission check compares.
    const PERMISSION_BITS: u32 = 0o777;
    /// A file mode other users can read.
    const SHARED_FILE: u32 = 0o644;

    /// The file appears whole, replacing what was there.
    #[test]
    fn a_text_file_is_written_whole() {
        let scratch = TempDir::new().unwrap();
        let path = scratch.path().join("muster.yml");

        write_atomically(&path, "old\n").unwrap();
        write_atomically(&path, "version: 1\n").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "version: 1\n");
    }

    /// A private file is its owner's alone, even replacing a readable one.
    #[test]
    fn a_private_file_is_the_owners_alone() {
        let scratch = TempDir::new().unwrap();
        let path = scratch.path().join("muster.yml");
        write_atomically(&path, "old\n").unwrap();
        fs::set_permissions(&path, Permissions::from_mode(SHARED_FILE)).unwrap();

        write_privately(&path, "secret\n").unwrap();

        let mode = fs::metadata(&path).unwrap().permissions().mode() & PERMISSION_BITS;
        assert_eq!(mode, PRIVATE_FILE_MODE);
        assert_eq!(fs::read_to_string(&path).unwrap(), "secret\n");
    }
}
