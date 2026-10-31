use std::{
    fs::{self, File},
    io::{self, Write},
    path::Path,
};

use nutype::nutype;
use uuid::Uuid;

use crate::adapter::bounded_read::read_bounded;

/// The file inside a lock folder that names muster as its holder. Claude
/// Code's lock never looks inside its folder, so the file tells muster's
/// folder apart from one made again in its place, however the file system
/// keeps times or reuses inodes.
pub(super) const TOKEN_FILE: &str = "muster.token";
/// The most of a token file muster reads: well above a token's length.
const MAX_TOKEN_SIZE: u64 = 256;

/// The random token muster leaves in a lock folder it made.
#[nutype(derive(Debug, Clone, PartialEq, Eq, AsRef))]
pub(super) struct LockToken(String);

impl LockToken {
    /// A token no other holder has.
    pub(super) fn fresh() -> Self {
        Self::new(Uuid::new_v4().to_string())
    }

    /// Leaves the token in `folder`, a lock folder muster just made.
    ///
    /// # Errors
    /// Returns the failure to write it, or [`io::ErrorKind::AlreadyExists`]
    /// when the folder already holds a token.
    pub(super) fn leave_in(&self, folder: &Path) -> io::Result<()> {
        let mut file = File::create_new(folder.join(TOKEN_FILE))?;
        file.write_all(self.as_ref().as_bytes())
    }

    /// Whether `folder` holds this token, so the lock is muster's still.
    pub(super) fn is_in(&self, folder: &Path) -> bool {
        read_bounded(&folder.join(TOKEN_FILE), MAX_TOKEN_SIZE)
            .is_ok_and(|text| text == *self.as_ref())
    }

    /// Takes the token out of `folder` and removes the folder, only while
    /// the folder holds this token.
    pub(super) fn release(&self, folder: &Path) {
        if self.is_in(folder) {
            clear(folder);
        }
    }
}

/// Removes the lock folder `folder`, taking out a token a muster that died
/// left in it first; a folder holding anything else stays.
pub(super) fn clear(folder: &Path) {
    let _ = fs::remove_file(folder.join(TOKEN_FILE));
    let _ = fs::remove_dir(folder);
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    /// A token is muster's only in the folder it was left in, is released
    /// with its folder, and leaves another holder's folder alone.
    #[test]
    fn a_token_marks_only_its_own_folder() {
        let dir = TempDir::new().unwrap();
        let folder = dir.path().join("x.lock");
        fs::create_dir(&folder).unwrap();
        let ours = LockToken::fresh();
        let theirs = LockToken::fresh();
        assert_ne!(ours, theirs);
        ours.leave_in(&folder).unwrap();
        assert!(ours.is_in(&folder) && !theirs.is_in(&folder));
        assert!(theirs.leave_in(&folder).is_err(), "one token per folder");

        theirs.release(&folder);
        assert!(folder.is_dir(), "another token's folder stays");
        ours.release(&folder);
        assert!(!folder.exists());
        assert!(!ours.is_in(&folder));
    }
}
