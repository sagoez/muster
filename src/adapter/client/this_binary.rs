use std::{
    env,
    ffi::{OsStr, OsString},
    fs, io,
    os::unix::{ffi::OsStrExt, fs::PermissionsExt},
    path::{Path, PathBuf},
};

use crate::constants::{APP_NAME, PATH_ENV};

/// What Linux appends to the path of a running binary that was replaced or
/// removed since it started.
const DELETED_SUFFIX: &str = " (deleted)";
/// The permission bits of a file anyone may run.
const RUNNABLE_BITS: u32 = 0o111;

/// This muster binary as it is now, for what runs it again: its own path,
/// or when that cannot be read, `muster` as `PATH` finds it, else the bare
/// name. On Linux a binary replaced or removed since it started reads as
/// its path marked deleted.
pub fn this_binary() -> PathBuf {
    located(env::current_exe(), env::var_os(PATH_ENV))
}

/// This muster binary by the path a person would type for it: the first
/// `muster` on `PATH` when that is this same binary, kept as `PATH` names it
/// so no versioned install path is written down, else its own path without
/// the mark Linux gives a replaced binary.
pub fn binary_as_typed() -> PathBuf {
    as_typed(env::current_exe(), env::var_os(PATH_ENV))
}

/// The binary a pane can run for `binary`: `binary` when it is a file, or
/// for one Linux marks as replaced, the file now at its path. None for a
/// path that is not absolute or names no file.
pub fn runnable(binary: &Path) -> Option<PathBuf> {
    if !binary.is_absolute() {
        return None;
    }
    if binary.is_file() {
        return Some(binary.to_path_buf());
    }
    let current = unmarked(binary)?;
    current.is_file().then_some(current)
}

/// The first runnable file named `name` in the absolute folders of `path`,
/// a `PATH` value; a relative folder is skipped, as it names a different
/// file from every working folder.
pub fn first_on_path(path: &OsStr, name: &str) -> Option<PathBuf> {
    env::split_paths(path)
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join(name))
        .find(|candidate| {
            fs::metadata(candidate)
                .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & RUNNABLE_BITS != 0)
        })
}

/// `current`, the path the OS gave for this binary, or when it gave none,
/// `muster` in the first of the `path` folders holding it, else the bare
/// name.
fn located(current: io::Result<PathBuf>, path: Option<OsString>) -> PathBuf {
    current
        .ok()
        .or_else(|| first_on_path(&path?, APP_NAME))
        .unwrap_or_else(|| PathBuf::from(APP_NAME))
}

/// The path to write for this binary, given `current`, the path the OS gave
/// for it, and `path`, a `PATH` value.
fn as_typed(current: io::Result<PathBuf>, path: Option<OsString>) -> PathBuf {
    let first = path.and_then(|path| first_on_path(&path, APP_NAME));
    let Ok(own) = current else {
        return first.unwrap_or_else(|| PathBuf::from(APP_NAME));
    };
    let own = unmarked(&own).unwrap_or(own);
    match first {
        Some(first) if same_file(&first, &own) => first,
        _ => own,
    }
}

/// `binary` without the mark Linux gives a replaced binary, or none when it
/// carries no mark.
fn unmarked(binary: &Path) -> Option<PathBuf> {
    let kept = binary
        .as_os_str()
        .as_bytes()
        .strip_suffix(DELETED_SUFFIX.as_bytes())?;
    Some(PathBuf::from(OsStr::from_bytes(kept)))
}

/// Whether `one` and `other` lead to the same file once every link is
/// followed.
fn same_file(one: &Path, other: &Path) -> bool {
    matches!(
        (fs::canonicalize(one), fs::canonicalize(other)),
        (Ok(one), Ok(other)) if one == other
    )
}

#[cfg(test)]
mod tests {
    use std::{io::ErrorKind, os::unix::fs::symlink};

    use tempfile::TempDir;

    use super::*;

    /// A mode anyone may run.
    const RUNNABLE_MODE: u32 = 0o755;
    /// A mode nobody may run.
    const UNRUNNABLE_MODE: u32 = 0o644;

    /// A folder holding a file `muster` with `mode`.
    fn folder_with_muster(mode: u32) -> TempDir {
        let dir = TempDir::new().unwrap();
        let binary = dir.path().join(APP_NAME);
        fs::write(&binary, "").unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(mode)).unwrap();
        dir
    }

    /// The OS's path wins; without it, `muster` is found in the first
    /// absolute `PATH` folder holding a runnable one, and without that the
    /// bare name is left, which no pane accepts.
    #[test]
    fn without_its_own_path_the_binary_is_found_on_path() {
        let unreadable = || Err(io::Error::from(ErrorKind::NotFound));
        let runnable_dir = folder_with_muster(RUNNABLE_MODE);
        let unrunnable_dir = folder_with_muster(UNRUNNABLE_MODE);
        let own = PathBuf::from("/opt/muster/bin/muster");

        assert_eq!(
            located(Ok(own.clone()), Some(runnable_dir.path().into())),
            own
        );
        let path = env::join_paths([
            Path::new("relative"),
            unrunnable_dir.path(),
            runnable_dir.path(),
        ])
        .unwrap();
        let found = located(unreadable(), Some(path));
        assert_eq!(found, runnable_dir.path().join(APP_NAME));
        assert_eq!(runnable(&found), Some(found));

        let bare = located(unreadable(), Some(unrunnable_dir.path().into()));
        assert_eq!(bare, PathBuf::from(APP_NAME));
        assert_eq!(located(unreadable(), None), PathBuf::from(APP_NAME));
        assert_eq!(runnable(&bare), None);
    }

    /// A binary Linux marks as replaced runs as the file now at its path;
    /// with nothing there, or for a path that is not absolute, none runs.
    #[test]
    fn a_replaced_binary_runs_as_the_file_now_at_its_path() {
        let dir = folder_with_muster(RUNNABLE_MODE);
        let current = dir.path().join(APP_NAME);
        let replaced = PathBuf::from(format!("{}{DELETED_SUFFIX}", current.display()));
        let removed = dir.path().join(format!("gone{DELETED_SUFFIX}"));

        assert_eq!(runnable(&current), Some(current.clone()));
        assert_eq!(runnable(&replaced), Some(current));
        assert_eq!(runnable(&removed), None);
        assert_eq!(runnable(&dir.path().join("missing")), None);
        assert_eq!(runnable(Path::new(APP_NAME)), None);
    }

    /// A link on `PATH` to this very binary is written as the link, so a
    /// versioned target is not; a `PATH` muster that is another binary, or
    /// none, leaves this binary's own path, unmarked when Linux marked it.
    #[test]
    fn the_typed_path_is_the_path_link_to_this_binary() {
        let installs = folder_with_muster(RUNNABLE_MODE);
        let own = installs.path().join(APP_NAME);
        let bin = TempDir::new().unwrap();
        symlink(&own, bin.path().join(APP_NAME)).unwrap();
        let other = folder_with_muster(RUNNABLE_MODE);

        assert_eq!(
            as_typed(Ok(own.clone()), Some(bin.path().into())),
            bin.path().join(APP_NAME)
        );
        let marked = PathBuf::from(format!("{}{DELETED_SUFFIX}", own.display()));
        assert_eq!(
            as_typed(Ok(marked.clone()), Some(bin.path().into())),
            bin.path().join(APP_NAME),
            "a replaced binary is the file now at its path"
        );
        assert_eq!(as_typed(Ok(marked), None), own);
        assert_eq!(
            as_typed(Ok(own.clone()), Some(other.path().into())),
            own,
            "the first muster on PATH is another binary"
        );
        let later = env::join_paths([other.path(), bin.path()]).unwrap();
        assert_eq!(as_typed(Ok(own.clone()), Some(later)), own);
        assert_eq!(
            as_typed(Err(io::Error::from(ErrorKind::NotFound)), None),
            PathBuf::from(APP_NAME)
        );
    }
}
