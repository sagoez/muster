//! Reading a text file someone else wrote without trusting its kind or size:
//! a FIFO or device is never opened and waited on, and a huge file is never
//! read into memory.

use std::{
    fs::{self, File},
    io::{self, Read},
    path::Path,
};

/// The byte order mark an editor may put before UTF-8 text.
const BYTE_ORDER_MARK: char = '\u{feff}';
/// Why a path that is not a regular file is refused.
const NOT_A_FILE: &str = "not a regular file";
/// Why a file over the cap is refused.
const TOO_LARGE: &str = "file is too large";

/// The UTF-8 text of `path`, without a leading byte order mark, when it is a
/// regular file of at most `max` bytes. The kind is checked before opening,
/// so a FIFO is never opened and waited on.
///
/// # Errors
/// Returns an [`io::Error`] of kind [`io::ErrorKind::InvalidInput`] for a
/// path that is not a regular file, [`io::ErrorKind::FileTooLarge`] for one
/// over `max` bytes, or the failure of the read itself.
pub fn read_bounded(path: &Path, max: u64) -> io::Result<String> {
    let metadata = fs::metadata(path)?;
    if !metadata.is_file() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, NOT_A_FILE));
    }
    if metadata.len() > max {
        return Err(io::Error::new(io::ErrorKind::FileTooLarge, TOO_LARGE));
    }
    read_capped(File::open(path)?, max)
}

/// The UTF-8 text `reader` yields, without a leading byte order mark,
/// refused once it runs past `max` bytes.
///
/// # Errors
/// Returns an [`io::Error`] of kind [`io::ErrorKind::FileTooLarge`] past
/// `max` bytes, or the failure of the read itself, including text that is
/// not UTF-8.
pub fn read_capped(reader: impl Read, max: u64) -> io::Result<String> {
    let mut bytes = Vec::new();
    reader.take(max.saturating_add(1)).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max {
        return Err(io::Error::new(io::ErrorKind::FileTooLarge, TOO_LARGE));
    }
    let text = String::from_utf8(bytes)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    Ok(match text.strip_prefix(BYTE_ORDER_MARK) {
        Some(rest) => rest.to_string(),
        None => text,
    })
}

#[cfg(test)]
mod tests {
    use rustix::fs::{CWD, FileType, Mode, mknodat};
    use tempfile::TempDir;

    use super::*;

    /// A file within the cap reads, its byte order mark dropped; one past it,
    /// a folder and a FIFO are refused without being waited on.
    #[test]
    fn only_a_small_regular_file_is_read() {
        let dir = TempDir::new().unwrap();
        let small = dir.path().join("small");
        fs::write(&small, "\u{feff}{\"a\": 1}").unwrap();
        assert_eq!(read_bounded(&small, 64).unwrap(), "{\"a\": 1}");

        let large = dir.path().join("large");
        fs::write(&large, "x".repeat(65)).unwrap();
        assert_eq!(
            read_bounded(&large, 64).unwrap_err().kind(),
            io::ErrorKind::FileTooLarge
        );
        assert_eq!(
            read_bounded(dir.path(), 64).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );

        let fifo = dir.path().join("fifo");
        mknodat(CWD, &fifo, FileType::Fifo, Mode::RUSR | Mode::WUSR, 0).unwrap();
        assert_eq!(
            read_bounded(&fifo, 64).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
    }

    /// A stream is cut off and refused once it passes the cap.
    #[test]
    fn a_stream_past_the_cap_is_refused() {
        assert_eq!(read_capped(&b"abc"[..], 3).unwrap(), "abc");
        assert_eq!(
            read_capped(&b"abcd"[..], 3).unwrap_err().kind(),
            io::ErrorKind::FileTooLarge
        );
    }
}
