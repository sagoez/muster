use std::{
    ffi::OsString,
    fs, io,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

use nutype::nutype;
use uuid::Uuid;

use crate::adapter::bounded_read::read_bounded;

/// What muster appends to a lock folder's path for the file beside it that
/// names muster as the folder's holder. The folder itself stays empty, so
/// Claude Code's `rmdir` can always take a stale one over.
const TOKEN_SUFFIX: &str = ".muster";
/// The most of a token file muster reads: well above a token's length.
const MAX_TOKEN_SIZE: u64 = 256;
/// Between a token's random part and the identity of the folder it names.
const TOKEN_SEPARATOR: char = '\n';
/// Stands for a creation time the file system does not keep.
const NO_BIRTH: &str = "-";

/// muster's claim on one lock folder, kept in the token file beside it: a
/// random part no other holder has, and the identity of the folder muster
/// made, so a folder removed and made again in its place, even one reusing
/// the inode, is told apart by its creation time.
#[nutype(derive(Debug, Clone, PartialEq, Eq, AsRef))]
pub(super) struct LockToken(String);

impl LockToken {
    /// Claims `folder`, a lock folder muster just made, by writing a fresh
    /// token beside it in place of any a muster that died left.
    ///
    /// # Errors
    /// Returns the failure to look at the folder or write the token.
    pub(super) fn claim(folder: &Path) -> io::Result<Self> {
        let token = Self::new(format!(
            "{}{TOKEN_SEPARATOR}{}",
            Uuid::new_v4(),
            identity(folder)?
        ));
        let file = token_file(folder);
        fs::write(&file, token.as_ref()).inspect_err(|_| {
            let _ = fs::remove_file(&file);
        })?;
        Ok(token)
    }

    /// Whether the lock is muster's still: the token beside `folder` is
    /// this one, and `folder` is the very folder it names.
    pub(super) fn holds(&self, folder: &Path) -> bool {
        self.is_beside(folder)
            && self
                .as_ref()
                .split_once(TOKEN_SEPARATOR)
                .is_some_and(|(_, named)| identity(folder).is_ok_and(|now| now == named))
    }

    /// Lets the lock go: the token and then `folder` are removed while the
    /// lock is muster's; once another writer took the folder over, only
    /// the token, still this one, is removed and the folder kept.
    pub(super) fn release(&self, folder: &Path) {
        let held = self.holds(folder);
        if held || self.is_beside(folder) {
            let _ = fs::remove_file(token_file(folder));
        }
        if held {
            let _ = fs::remove_dir(folder);
        }
    }

    /// Whether the token file beside `folder` holds this token.
    fn is_beside(&self, folder: &Path) -> bool {
        read_bounded(&token_file(folder), MAX_TOKEN_SIZE).is_ok_and(|text| text == *self.as_ref())
    }
}

/// The token file beside the lock folder `folder`.
pub(super) fn token_file(folder: &Path) -> PathBuf {
    let mut file = OsString::from(folder.as_os_str());
    file.push(TOKEN_SUFFIX);
    PathBuf::from(file)
}

/// The identity of the folder at `folder`: its device, inode and, where the
/// file system keeps it, creation time.
///
/// # Errors
/// Returns the failure to look at it, or [`io::ErrorKind::NotADirectory`]
/// for something other than a folder.
fn identity(folder: &Path) -> io::Result<String> {
    let metadata = fs::symlink_metadata(folder)?;
    if !metadata.is_dir() {
        return Err(io::ErrorKind::NotADirectory.into());
    }
    let born = metadata
        .created()
        .ok()
        .and_then(|born| born.duration_since(UNIX_EPOCH).ok())
        .map_or_else(|| NO_BIRTH.to_owned(), |born| born.as_nanos().to_string());
    Ok(format!("{} {} {born}", metadata.dev(), metadata.ino()))
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    /// A claim holds only the folder it was made on, leaves the folder
    /// empty, and is released with it; a claim on a folder made again in
    /// its place no longer holds, and its release keeps the new folder.
    #[test]
    fn a_claim_holds_only_its_own_folder() {
        let dir = TempDir::new().unwrap();
        let folder = dir.path().join("x.lock");
        fs::create_dir(&folder).unwrap();
        let ours = LockToken::claim(&folder).unwrap();
        assert!(ours.holds(&folder));
        assert_eq!(fs::read_dir(&folder).unwrap().count(), 0, "kept empty");
        ours.release(&folder);
        assert!(!folder.exists() && !token_file(&folder).exists());

        fs::create_dir(&folder).unwrap();
        let lost = LockToken::claim(&folder).unwrap();
        fs::remove_dir(&folder).unwrap();
        fs::create_dir(&folder).unwrap();
        assert!(!lost.holds(&folder), "a folder made again is not ours");
        lost.release(&folder);
        assert!(folder.is_dir(), "the new holder's folder stays");
        assert!(!token_file(&folder).exists(), "our stale token goes");

        let theirs = LockToken::claim(&folder).unwrap();
        lost.release(&folder);
        assert!(theirs.holds(&folder), "another claim's token stays");
    }
}
