use std::{
    ffi::OsString,
    fs::{self, OpenOptions},
    io::{self, Write},
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    thread,
    time::{Duration, UNIX_EPOCH},
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
/// How many more times a look at the lock that failed for a moment is
/// made before the lock counts as lost.
const HOLD_RETRIES: u32 = 3;
/// How long muster waits before looking at the lock again.
const HOLD_RETRY_PAUSE: Duration = Duration::from_millis(10);

/// Reads the token file at a path.
type TokenReader = fn(&Path) -> io::Result<String>;

/// Runs between removing a stale token file and creating the new one.
type BetweenSteps = fn(&Path);

/// muster's claim on one lock folder, kept in the token file beside it: a
/// random part no other holder has, and the identity of the folder muster
/// made, so a folder removed and made again in its place, even one reusing
/// the inode, is told apart by its creation time.
#[nutype(derive(Debug, Clone, PartialEq, Eq, AsRef))]
pub(super) struct LockToken(String);

impl LockToken {
    /// Claims `folder`, a lock folder muster just made, by removing any
    /// token a muster that died left beside it and creating a fresh one.
    ///
    /// # Errors
    /// Returns the failure to look at the folder or write the token, of
    /// kind [`io::ErrorKind::AlreadyExists`] when another writer made a
    /// token file in between, which is then left as it is.
    pub(super) fn claim(folder: &Path) -> io::Result<Self> {
        Self::claim_between(folder, |_| {})
    }

    /// As [`Self::claim`], running `between` once the stale token is gone
    /// and before the new one is created.
    ///
    /// # Errors
    /// As [`Self::claim`].
    fn claim_between(folder: &Path, between: BetweenSteps) -> io::Result<Self> {
        let token = Self::new(format!(
            "{}{TOKEN_SEPARATOR}{}",
            Uuid::new_v4(),
            identity(folder)?
        ));
        let file = token_file(folder);
        match fs::remove_file(&file) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
            _ => {},
        }
        between(&file);
        // `create_new` so muster never writes through a token file another
        // writer made since the stale one went.
        let mut created = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&file)?;
        created
            .write_all(token.as_ref().as_bytes())
            .inspect_err(|_| {
                let _ = fs::remove_file(&file);
            })?;
        Ok(token)
    }

    /// Whether the lock is muster's still: the token beside `folder` is
    /// this one, and `folder` is the very folder it names. A look that
    /// fails for a moment is made again before the lock counts as lost.
    pub(super) fn holds(&self, folder: &Path) -> bool {
        self.holds_read_by(folder, read_token)
    }

    /// As [`Self::holds`], reading the token file with `read`.
    fn holds_read_by(&self, folder: &Path, read: TokenReader) -> bool {
        let mut retries = 0;
        loop {
            match self.look(folder, read) {
                Ok(held) => return held,
                Err(error) if is_transient(&error) && retries < HOLD_RETRIES => {
                    retries += 1;
                    thread::sleep(HOLD_RETRY_PAUSE);
                },
                Err(_) => return false,
            }
        }
    }

    /// One look at whether the lock is muster's, read with `read`.
    ///
    /// # Errors
    /// Returns the failure to read the token file or look at `folder`.
    fn look(&self, folder: &Path, read: TokenReader) -> io::Result<bool> {
        if read(&token_file(folder))? != *self.as_ref() {
            return Ok(false);
        }
        let Some((_, named)) = self.as_ref().split_once(TOKEN_SEPARATOR) else {
            return Ok(false);
        };
        Ok(identity(folder)? == named)
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
        read_token(&token_file(folder)).is_ok_and(|text| text == *self.as_ref())
    }
}

/// The token file beside the lock folder `folder`.
pub(super) fn token_file(folder: &Path) -> PathBuf {
    let mut file = OsString::from(folder.as_os_str());
    file.push(TOKEN_SUFFIX);
    PathBuf::from(file)
}

/// The text of the token file at `file`.
///
/// # Errors
/// Returns the failure to read it, as [`read_bounded`] does.
fn read_token(file: &Path) -> io::Result<String> {
    read_bounded(file, MAX_TOKEN_SIZE)
}

/// Whether `error` may pass on its own: an interrupted or blocked call, or
/// the token file or folder missing for the moment another writer's
/// takeover check takes.
fn is_transient(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock | io::ErrorKind::NotFound
    )
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
    use std::sync::atomic::{AtomicU32, Ordering};

    use tempfile::TempDir;

    use super::*;

    /// What another writer puts in a token file of its own.
    const THEIRS: &str = "theirs";

    /// How many reads the flaky reader has been asked for.
    static FLAKY_READS: AtomicU32 = AtomicU32::new(0);
    /// How many reads the reader of another's token has been asked for.
    static THEIR_READS: AtomicU32 = AtomicU32::new(0);
    /// How many reads the reader that never recovers has been asked for.
    static FAILING_READS: AtomicU32 = AtomicU32::new(0);

    /// Reads the token file, failing first as interrupted, then as would
    /// block, then as missing, as a busy file system and another writer's
    /// takeover check may.
    fn flaky(file: &Path) -> io::Result<String> {
        match FLAKY_READS.fetch_add(1, Ordering::SeqCst) {
            0 => Err(io::ErrorKind::Interrupted.into()),
            1 => Err(io::ErrorKind::WouldBlock.into()),
            2 => Err(io::ErrorKind::NotFound.into()),
            _ => read_token(file),
        }
    }

    /// Reads another writer's token in place of muster's.
    fn their_token(_: &Path) -> io::Result<String> {
        THEIR_READS.fetch_add(1, Ordering::SeqCst);
        Ok(THEIRS.to_owned())
    }

    /// Fails as interrupted every time.
    fn always_interrupted(_: &Path) -> io::Result<String> {
        FAILING_READS.fetch_add(1, Ordering::SeqCst);
        Err(io::ErrorKind::Interrupted.into())
    }

    /// Makes another writer's token file at `file`.
    fn their_claim(file: &Path) {
        fs::write(file, THEIRS).unwrap();
    }

    /// A lock folder made in a scratch folder.
    fn lock_folder(dir: &TempDir) -> PathBuf {
        let folder = dir.path().join("x.lock");
        fs::create_dir(&folder).unwrap();
        folder
    }

    /// A claim holds only the folder it was made on, leaves the folder
    /// empty, and is released with it; a claim on a folder made again in
    /// its place no longer holds, and its release keeps the new folder.
    #[test]
    fn a_claim_holds_only_its_own_folder() {
        let dir = TempDir::new().unwrap();
        let folder = lock_folder(&dir);
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

    /// A stale token a muster that died left is replaced by the claim.
    #[test]
    fn a_stale_token_is_replaced() {
        let dir = TempDir::new().unwrap();
        let folder = lock_folder(&dir);
        their_claim(&token_file(&folder));
        let ours = LockToken::claim(&folder).unwrap();
        assert!(ours.holds(&folder));
        assert_eq!(read_token(&token_file(&folder)).unwrap(), *ours.as_ref());
    }

    /// A token file another writer makes once the stale one is gone makes
    /// muster back off, and theirs is left as they wrote it.
    #[test]
    fn a_token_made_in_between_is_never_written_through() {
        let dir = TempDir::new().unwrap();
        let folder = lock_folder(&dir);
        let file = token_file(&folder);
        their_claim(&file);
        let refused = LockToken::claim_between(&folder, their_claim).unwrap_err();
        assert_eq!(refused.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read_to_string(&file).unwrap(), THEIRS);
    }

    /// A look that fails for a moment is made again and the lock is still
    /// muster's; another writer's token ends the look at once; a failure
    /// that never passes counts as lost after the retries.
    #[test]
    fn a_transient_failure_is_retried_but_a_loss_is_not() {
        let dir = TempDir::new().unwrap();
        let folder = lock_folder(&dir);
        let ours = LockToken::claim(&folder).unwrap();

        assert!(ours.holds_read_by(&folder, flaky), "held through the blips");
        assert_eq!(FLAKY_READS.load(Ordering::SeqCst), HOLD_RETRIES + 1);

        assert!(!ours.holds_read_by(&folder, their_token));
        assert_eq!(THEIR_READS.load(Ordering::SeqCst), 1, "not retried");

        assert!(!ours.holds_read_by(&folder, always_interrupted));
        assert_eq!(FAILING_READS.load(Ordering::SeqCst), HOLD_RETRIES + 1);
    }
}
