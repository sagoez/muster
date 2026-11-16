use std::{
    fs, io,
    path::{Path, PathBuf},
};

use getset::{Getters, WithSetters};
use muster_domain::value::{ProcessName, ProjectRoot};
use muster_os::private_dir::{create_private_dir_all, is_owned_private_dir, tmp_socket_dir};
use typed_builder::TypedBuilder;

use crate::{
    holder::{
        error::HolderError,
        launch::{
            id::{HOLDER_ID_DIGITS, HolderId},
            socket_place::SocketPlace,
        },
    },
    ipc::fits_a_socket,
    key::{hash_key, path_key},
};

/// File extension of a holder socket.
const SOCKET_EXTENSION: &str = "sock";
/// File extension of a holder's output log.
const LOG_EXTENSION: &str = "log";
/// Directory under the state directory holding every holder's files.
const HOLDERS_DIR: &str = "holders";
/// Directory of holder sockets and handover files, private to the user.
const SOCKETS_DIR: &str = "sock";
/// Directory of holder records.
const RECORDS_DIR: &str = "records";
/// Directory of each process's output log, kept outside the holders' own
/// files because a log outlives every holder that wrote it.
const LOGS_DIR: &str = "logs";
/// Directory under the holders' directory where an earlier muster kept one
/// log per holder.
const LEGACY_LOGS_DIR: &str = "logs";
/// Punctuation a process name keeps in its log file name.
const LOG_NAME_KEPT: &[u8] = b"-_ ";
/// Longest log file stem kept as written, well inside the 255-byte limit.
const LOG_STEM_MAX: usize = 200;
/// Hex digits of the name's hash a log file name ends with.
const LOG_NAME_HASH_DIGITS: usize = 8;
/// Between a log file name's encoded process name and its hash.
const LOG_HASH_SEPARATOR: char = '.';
/// A hex digit standing for each of a generated holder id's, when only its
/// length matters.
const GENERATED_ID_CHAR: &str = "0";
/// Folder of a test's holders standing in for `/tmp/muster-<uid>`.
#[cfg(any(test, feature = "testing"))]
const SCRATCH_FALLBACK_ROOT: &str = "run";

/// Where holders keep their sockets, records and logs.
#[derive(Debug, Clone, Getters, WithSetters, TypedBuilder)]
pub struct HolderDirs {
    /// The holder sockets' folder inside the state directory.
    #[getset(get = "pub")]
    state_sockets: PathBuf,
    /// The holder sockets' folder under `/tmp/muster-<uid>`, named for the
    /// state directory, for when a socket in `state_sockets` would not fit a
    /// Unix socket address.
    #[getset(get = "pub")]
    fallback_sockets: PathBuf,
    /// Which of the two new holders bind their sockets in.
    #[getset(get = "pub", set_with = "pub")]
    place: SocketPlace,
    /// Directory of holder records.
    #[getset(get = "pub")]
    records: PathBuf,
    /// Directory of child output logs.
    #[getset(get = "pub")]
    logs: PathBuf,
}

impl HolderDirs {
    /// The holder directories a service owning `state_dir` keeps, with the
    /// fallback sockets' folder in `/tmp/muster-<uid>`, which, unlike a
    /// runtime directory, no logout removes and whose paths always fit.
    pub fn placed(state_dir: &Path) -> Self {
        Self::placed_in(state_dir, &tmp_socket_dir())
    }

    /// The holder directories a service owning `state_dir` keeps, with the
    /// fallback sockets' folder in `fallback_root`. Holder sockets go in the
    /// state directory when one fits a Unix socket address there, else in a
    /// folder of `fallback_root` named for the state directory, so two state
    /// directories never share it. The choice is made here, once, so every
    /// caller agrees on it.
    pub fn placed_in(state_dir: &Path, fallback_root: &Path) -> Self {
        let holders = state_dir.join(HOLDERS_DIR);
        let state_sockets = holders.join(SOCKETS_DIR);
        let place = if fits_a_socket(&longest_socket_in(&state_sockets)) {
            SocketPlace::State
        } else {
            SocketPlace::Fallback
        };
        Self::builder()
            .state_sockets(state_sockets)
            .fallback_sockets(fallback_root.join(path_key(state_dir)))
            .place(place)
            .records(holders.join(RECORDS_DIR))
            .logs(state_dir.join(LOGS_DIR))
            .build()
    }

    /// Holder dirs for a test: sockets, records and a fallback folder under
    /// `holders`, new sockets in its `sock` folder, and logs in `logs`.
    #[cfg(any(test, feature = "testing"))]
    pub fn scratch(holders: &Path, logs: PathBuf) -> Self {
        Self::builder()
            .state_sockets(holders.join(SOCKETS_DIR))
            .fallback_sockets(holders.join(SCRATCH_FALLBACK_ROOT).join(HOLDERS_DIR))
            .place(SocketPlace::State)
            .records(holders.join(RECORDS_DIR))
            .logs(logs)
            .build()
    }

    /// The folder new holders bind their sockets in, private to its owner.
    pub fn sockets(&self) -> &PathBuf {
        match self.place {
            SocketPlace::State => &self.state_sockets,
            SocketPlace::Fallback => &self.fallback_sockets,
        }
    }

    /// Every folder a holder socket of this state directory may be in, the
    /// one new holders bind in first.
    pub fn socket_folders(&self) -> [&PathBuf; 2] {
        match self.place {
            SocketPlace::State => [&self.state_sockets, &self.fallback_sockets],
            SocketPlace::Fallback => [&self.fallback_sockets, &self.state_sockets],
        }
    }

    /// Refuses `folder`, one of [`Self::socket_folders`], unless it is a
    /// real directory this user owns that no other user can enter, and, for
    /// the fallback folder, unless its parent in a shared root is too.
    ///
    /// # Errors
    /// Returns [`HolderError::InsecureSocketDir`] naming the directory that
    /// is not private, or the I/O error from inspecting one, of kind
    /// [`io::ErrorKind::NotFound`] when it is absent.
    pub fn check_socket_folder(&self, folder: &Path) -> Result<(), HolderError> {
        if folder == self.fallback_sockets
            && let Some(root) = folder.parent()
        {
            check_private_dir(root)?;
        }
        check_private_dir(folder)
    }

    /// Removes the per-holder logs an earlier muster kept under the holders'
    /// own directory, which nothing reads any more. An absent directory is
    /// not an error.
    ///
    /// # Errors
    /// Returns the I/O error from removing it.
    pub fn remove_legacy_logs(&self) -> io::Result<()> {
        let Some(holders) = self.records.parent() else {
            return Ok(());
        };
        match fs::remove_dir_all(holders.join(LEGACY_LOGS_DIR)) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            removed => removed,
        }
    }

    /// The socket path of holder `id`.
    pub fn socket_path(&self, id: &HolderId) -> PathBuf {
        file_in(self.sockets(), id, SOCKET_EXTENSION)
    }

    /// Whether `path` names a holder socket in one of
    /// [`Self::socket_folders`], as [`Self::socket_path`] makes them.
    pub fn is_socket(&self, path: &Path) -> bool {
        path.parent()
            .is_some_and(|parent| self.socket_folders().iter().any(|folder| *folder == parent))
            && path.extension().is_some_and(|ext| ext == SOCKET_EXTENSION)
    }

    /// The log of the process `name` of the project at `project`: one file
    /// per process that every run appends to, so it can be read after the
    /// process exits. The project's folder is named by a stable hash of its
    /// root, which is already canonical, so every alias of it shares the log.
    pub fn process_log(&self, project: &ProjectRoot, name: &ProcessName) -> PathBuf {
        self.logs
            .join(path_key(project.as_ref()))
            .join(format!("{}.{LOG_EXTENSION}", log_file_stem(name)))
    }

    /// Creates the folder new holders bind in owner-only when absent, so a
    /// holder is only ever handed a private one. The fallback folder's
    /// parent lies in `/tmp`, which every user can write to, so it is made
    /// and checked first.
    ///
    /// # Errors
    /// Returns [`HolderError::InsecureSocketDir`] naming a directory that is
    /// not private to this user, or the I/O error from creating it.
    pub fn prepare_sockets(&self) -> Result<(), HolderError> {
        if self.place == SocketPlace::Fallback
            && let Some(root) = self.fallback_sockets.parent()
        {
            ensure_private_dir(root)?;
        }
        ensure_private_dir(self.sockets())
    }
}

/// The file named for `id` with `extension` inside `dir`.
fn file_in(dir: &Path, id: &HolderId, extension: &str) -> PathBuf {
    dir.join(id.as_ref()).with_extension(extension)
}

/// The longest socket path a generated holder id makes in `dir`.
fn longest_socket_in(dir: &Path) -> PathBuf {
    dir.join(GENERATED_ID_CHAR.repeat(HOLDER_ID_DIGITS))
        .with_extension(SOCKET_EXTENSION)
}

/// A file name stem for `name` that is safe on every supported file system:
/// ASCII letters, digits, `-`, `_` and spaces are kept, and every other byte
/// is percent-encoded, a leading dot included, so no name can climb out of
/// its folder or hide. A short hash of the exact name follows, so names that
/// differ only in case keep separate logs on a case-insensitive file system.
/// A name too long for a file name is replaced by its full hash.
fn log_file_stem(name: &ProcessName) -> String {
    let mut stem = String::new();
    for byte in name.as_ref().bytes() {
        if byte.is_ascii_alphanumeric() || LOG_NAME_KEPT.contains(&byte) {
            stem.push(char::from(byte));
        } else {
            stem.push_str(&format!("%{byte:02X}"));
        }
    }
    let hash = hash_key(name.as_ref().as_bytes());
    if stem.len() > LOG_STEM_MAX {
        return hash;
    }
    format!(
        "{stem}{LOG_HASH_SEPARATOR}{}",
        &hash[..LOG_NAME_HASH_DIGITS]
    )
}

/// Creates `dir` owner-only when absent, and refuses an existing one that is
/// not private to this user rather than changing a possibly shared
/// directory's mode.
///
/// # Errors
/// Returns [`HolderError::InsecureSocketDir`] when the directory is not
/// private, or the I/O error from creating or inspecting it.
pub fn ensure_private_dir(dir: &Path) -> Result<(), HolderError> {
    create_private_dir_all(dir)?;
    check_private_dir(dir)
}

/// Refuses `dir` unless it is a real directory, not a link, this user owns
/// and no other user can enter: the rule the service's socket directory
/// follows, so a holder trusts its socket's folder only as far as the
/// service does.
///
/// # Errors
/// Returns [`HolderError::InsecureSocketDir`] when it is not private, or the
/// I/O error from inspecting it, of kind [`io::ErrorKind::NotFound`] when it
/// is absent.
pub fn check_private_dir(dir: &Path) -> Result<(), HolderError> {
    if is_owned_private_dir(dir)? {
        Ok(())
    } else {
        Err(HolderError::InsecureSocketDir(dir.to_path_buf()))
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs::{self, Permissions},
        os::unix::fs::{PermissionsExt, symlink},
    };

    use muster_domain::value::PROCESS_NAME_MAX_CHARS;
    use muster_os::test_scratch::short_scratch;
    use rustix::process::geteuid;
    use tempfile::TempDir;

    use super::*;

    /// Prefix of a scratch directory a test binds or places sockets in.
    const SCRATCH_PREFIX: &str = "mp-";
    /// A directory mode other users can enter.
    const SHARED_DIR_MODE: u32 = 0o755;
    /// The mode a created socket directory gets.
    const PRIVATE_DIR_MODE: u32 = 0o700;
    /// Mode bits a permission check compares.
    const PERMISSION_BITS: u32 = 0o777;

    /// A folder name long enough that a holder socket inside it never fits
    /// a Unix socket address.
    const DEEP_DIR_CHARS: usize = 120;
    /// The scratch's stand-in for `/tmp/muster-<uid>`.
    const FALLBACK_ROOT: &str = "tmp";
    /// The root `/tmp/muster-<uid>` lies in.
    const TMP: &str = "/tmp";
    /// The name of `/tmp/muster-<uid>` before the uid.
    const TMP_SOCKET_PREFIX: &str = "muster-";
    /// A link a test plants in place of a folder.
    const PLANTED_LINK: &str = "planted";

    /// Holder dirs under `root`.
    fn dirs(root: &Path) -> HolderDirs {
        HolderDirs::scratch(root, root.join("logs"))
    }

    /// Holder dirs of a state directory in `scratch` too deep for a holder
    /// socket, with the fallback root in it too.
    fn deep_dirs(scratch: &Path) -> HolderDirs {
        HolderDirs::placed_in(
            &scratch.join("d".repeat(DEEP_DIR_CHARS)),
            &scratch.join(FALLBACK_ROOT),
        )
    }

    /// A short state directory keeps its holder sockets, and a state
    /// directory too deep for one puts them in a folder of the fallback
    /// root named for it; both folders are swept either way.
    #[test]
    fn holder_sockets_fall_back_only_when_they_do_not_fit() {
        let scratch = short_scratch(SCRATCH_PREFIX);
        let root = scratch.path().join(FALLBACK_ROOT);
        let short = scratch.path().join("state");

        let kept = HolderDirs::placed_in(&short, &root);
        let moved = deep_dirs(scratch.path());

        assert_eq!(*kept.place(), SocketPlace::State);
        assert_eq!(kept.sockets(), kept.state_sockets());
        assert!(kept.sockets().starts_with(&short));
        assert_eq!(*moved.place(), SocketPlace::Fallback);
        assert_eq!(moved.sockets(), moved.fallback_sockets());
        assert_eq!(moved.sockets().parent(), Some(root.as_path()));
        assert_ne!(kept.fallback_sockets(), moved.fallback_sockets());
        let id = HolderId::generate().unwrap();
        assert!(fits_a_socket(&kept.socket_path(&id)));
        assert!(fits_a_socket(&moved.socket_path(&id)));
        assert!(!fits_a_socket(&moved.state_sockets().join(id.as_ref())));
        assert_eq!(kept.socket_folders(), [
            kept.state_sockets(),
            kept.fallback_sockets()
        ]);
        assert_eq!(moved.socket_folders(), [
            moved.fallback_sockets(),
            moved.state_sockets()
        ]);
        for dirs in [&kept, &moved] {
            let other = dirs.socket_folders()[1].join(id.as_ref());
            assert!(dirs.is_socket(&other.with_extension(SOCKET_EXTENSION)));
        }
    }

    /// The fallback folder is in `/tmp/muster-<uid>`, which no logout
    /// removes, whatever runtime directory the service has, and a holder
    /// socket there always fits.
    #[test]
    fn the_fallback_folder_is_in_tmp_whatever_the_runtime_dir() {
        let deep = Path::new(TMP).join("d".repeat(DEEP_DIR_CHARS));

        let dirs = HolderDirs::placed(&deep);

        let root = dirs.fallback_sockets().parent().unwrap();
        assert_eq!(root, tmp_socket_dir());
        assert_eq!(root.parent(), Some(Path::new(TMP)));
        assert!(
            root.ends_with(format!("{TMP_SOCKET_PREFIX}{}", geteuid().as_raw())),
            "{}",
            root.display()
        );
        assert!(fits_a_socket(
            &dirs.socket_path(&HolderId::generate().unwrap())
        ));
    }

    /// The fallback folder is made private with its parent, and one whose
    /// parent other users can enter is refused, naming that parent.
    #[test]
    fn a_fallback_folder_is_readied_with_its_parent() {
        let scratch = short_scratch(SCRATCH_PREFIX);
        let dirs = deep_dirs(scratch.path());
        let root = scratch.path().join(FALLBACK_ROOT);

        dirs.prepare_sockets().unwrap();

        for dir in [&root, dirs.sockets()] {
            let mode = fs::metadata(dir).unwrap().permissions().mode() & PERMISSION_BITS;
            assert_eq!(mode, PRIVATE_DIR_MODE, "{}", dir.display());
        }
        dirs.check_socket_folder(dirs.sockets()).unwrap();
        fs::set_permissions(&root, Permissions::from_mode(SHARED_DIR_MODE)).unwrap();
        for refused in [
            dirs.prepare_sockets(),
            dirs.check_socket_folder(dirs.sockets()),
        ] {
            assert!(
                matches!(&refused, Err(HolderError::InsecureSocketDir(dir)) if *dir == root),
                "{refused:?}"
            );
        }
    }

    /// A link to a private directory is not a private directory: a holder
    /// and a sweep trust only a real one, as the service's socket check
    /// does.
    #[test]
    fn a_link_to_a_private_dir_is_refused() {
        let scratch = short_scratch(SCRATCH_PREFIX);
        let real = scratch.path().join("real");
        ensure_private_dir(&real).unwrap();
        let link = scratch.path().join(PLANTED_LINK);
        symlink(&real, &link).unwrap();

        assert!(matches!(
            ensure_private_dir(&link),
            Err(HolderError::InsecureSocketDir(_))
        ));
        assert!(matches!(
            check_private_dir(&link),
            Err(HolderError::InsecureSocketDir(_))
        ));
        check_private_dir(&real).unwrap();
    }

    #[test]
    fn the_socket_directory_is_created_private_and_a_shared_one_is_refused() {
        let scratch = TempDir::new().unwrap();
        let dirs = dirs(scratch.path());
        dirs.prepare_sockets().unwrap();
        let mode = fs::metadata(dirs.sockets()).unwrap().permissions().mode() & PERMISSION_BITS;
        assert_eq!(mode, PRIVATE_DIR_MODE);

        fs::set_permissions(dirs.sockets(), Permissions::from_mode(SHARED_DIR_MODE)).unwrap();
        assert!(matches!(
            dirs.prepare_sockets(),
            Err(HolderError::InsecureSocketDir(_))
        ));
    }

    /// The process `name`.
    fn name(name: &str) -> ProcessName {
        ProcessName::try_new(name).unwrap()
    }

    /// The project at `path`.
    fn root(path: PathBuf) -> ProjectRoot {
        ProjectRoot::try_new(path).unwrap()
    }

    /// One project's process logs share a stable folder, and each process has
    /// one log of its own.
    #[test]
    fn a_process_log_is_named_by_project_and_process() {
        let scratch = TempDir::new().unwrap();
        let dirs = HolderDirs::placed_in(scratch.path(), &scratch.path().join(FALLBACK_ROOT));
        let project = root(scratch.path().join("shop"));

        let log = dirs.process_log(&project, &name("dev server"));

        assert_eq!(
            log.file_name().unwrap().to_string_lossy(),
            format!("{}.log", log_file_stem(&name("dev server")))
        );
        assert!(log.starts_with(scratch.path().join(LOGS_DIR)));
        assert_eq!(log, dirs.process_log(&project, &name("dev server")));
        assert_ne!(log, dirs.process_log(&project, &name("api")));
        assert_ne!(
            log.parent(),
            dirs.process_log(&root(scratch.path().join("other")), &name("dev server"))
                .parent()
        );
    }

    /// A name with a separator, a leading dot or an accent cannot leave its
    /// folder, and a name too long for a file name is hashed.
    #[test]
    fn every_process_name_makes_a_safe_file_name() {
        assert_eq!(encoded(&name("../etc/passwd")), "%2E%2E%2Fetc%2Fpasswd");
        assert_eq!(encoded(&name("café")), "caf%C3%A9");
        assert_eq!(encoded(&name("dev_server-2")), "dev_server-2");
        let encodes_long = |accent: &str| name(&accent.repeat(PROCESS_NAME_MAX_CHARS));
        let long = encodes_long("\u{e9}");
        assert_eq!(log_file_stem(&long).len(), 16);
        assert_ne!(log_file_stem(&long), log_file_stem(&encodes_long("\u{e8}")));
    }

    /// Names that differ only in case get different log files, which a
    /// case-insensitive file system would otherwise merge into one.
    #[test]
    fn names_differing_only_in_case_get_different_logs() {
        let scratch = TempDir::new().unwrap();
        let dirs = HolderDirs::placed_in(scratch.path(), &scratch.path().join(FALLBACK_ROOT));
        let project = root(scratch.path().join("shop"));

        let upper = dirs.process_log(&project, &name("Web"));
        let lower = dirs.process_log(&project, &name("web"));

        assert_ne!(
            upper.to_string_lossy().to_lowercase(),
            lower.to_string_lossy().to_lowercase()
        );
        assert_eq!(encoded(&name("Web")), "Web");
        assert_eq!(
            log_file_stem(&name("web")).len(),
            "web".len() + 1 + LOG_NAME_HASH_DIGITS
        );
    }

    /// The encoded name a log file stem starts with, its hash cut off.
    fn encoded(name: &ProcessName) -> String {
        let stem = log_file_stem(name);
        let (encoded, hash) = stem.rsplit_once(LOG_HASH_SEPARATOR).unwrap();
        assert_eq!(hash.len(), LOG_NAME_HASH_DIGITS);
        encoded.to_string()
    }

    /// The legacy per-holder log directory is removed whole, and removing it
    /// again when it is gone is no error; the current logs are untouched.
    #[test]
    fn the_legacy_log_directory_is_removed_once() {
        let scratch = TempDir::new().unwrap();
        let dirs = HolderDirs::placed_in(scratch.path(), &scratch.path().join(FALLBACK_ROOT));
        let legacy = scratch.path().join(HOLDERS_DIR).join(LEGACY_LOGS_DIR);
        fs::create_dir_all(&legacy).unwrap();
        fs::write(legacy.join("0123abcd.log"), "old").unwrap();
        let current = dirs.process_log(&root(scratch.path().to_path_buf()), &name("api"));
        fs::create_dir_all(current.parent().unwrap()).unwrap();
        fs::write(&current, "new").unwrap();

        dirs.remove_legacy_logs().unwrap();
        dirs.remove_legacy_logs().unwrap();

        assert!(!legacy.exists());
        assert_eq!(fs::read_to_string(current).unwrap(), "new");
    }
}
