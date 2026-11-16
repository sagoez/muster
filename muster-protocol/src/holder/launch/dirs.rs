use std::{
    fs, io,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

use getset::Getters;
use muster_domain::value::{ProcessName, ProjectRoot};
use muster_os::private_dir::{GROUP_OTHER_BITS, create_private_dir_all, prepare_socket_dir};
use typed_builder::TypedBuilder;

use crate::{
    holder::{
        error::HolderError,
        launch::id::{HOLDER_ID_DIGITS, HolderId},
    },
    ipc::{fits_a_socket, fnv1a, path_key},
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

/// Where holders keep their sockets, records and logs.
#[derive(Debug, Clone, Getters, TypedBuilder)]
pub struct HolderDirs {
    /// Directory a new holder binds its socket in, private to its owner:
    /// either `state_sockets` or `runtime_sockets`.
    #[getset(get = "pub")]
    sockets: PathBuf,
    /// The holder sockets' folder inside the state directory.
    #[getset(get = "pub")]
    state_sockets: PathBuf,
    /// The holder sockets' folder in the service's socket directory, used
    /// when a socket in `state_sockets` would not fit a Unix socket address.
    #[getset(get = "pub")]
    runtime_sockets: PathBuf,
    /// Directory of holder records.
    #[getset(get = "pub")]
    records: PathBuf,
    /// Directory of child output logs.
    #[getset(get = "pub")]
    logs: PathBuf,
}

impl HolderDirs {
    /// The holder directories a service owning `state_dir`, with its socket
    /// in `socket_dir`, keeps. Holder sockets go in the state directory when
    /// one fits a Unix socket address there, else in a folder of
    /// `socket_dir` named for the state directory, so services of two state
    /// directories never share it. The choice is made here, once, so every
    /// caller agrees on it.
    pub fn placed(state_dir: &Path, socket_dir: &Path) -> Self {
        let holders = state_dir.join(HOLDERS_DIR);
        let state_sockets = holders.join(SOCKETS_DIR);
        let runtime_sockets = socket_dir.join(path_key(state_dir));
        let sockets = if fits_a_socket(&longest_socket_in(&state_sockets)) {
            state_sockets.clone()
        } else {
            runtime_sockets.clone()
        };
        Self::builder()
            .sockets(sockets)
            .state_sockets(state_sockets)
            .runtime_sockets(runtime_sockets)
            .records(holders.join(RECORDS_DIR))
            .logs(state_dir.join(LOGS_DIR))
            .build()
    }

    /// Every folder a holder socket of this state directory may be in, the
    /// one new holders bind in first.
    pub fn socket_folders(&self) -> Vec<&PathBuf> {
        let mut folders = vec![&self.sockets];
        folders.extend(
            [&self.state_sockets, &self.runtime_sockets]
                .into_iter()
                .filter(|folder| **folder != self.sockets),
        );
        folders
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
        file_in(&self.sockets, id, SOCKET_EXTENSION)
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

    /// Creates the socket directory owner-only when absent, so a holder is
    /// only ever handed a private one. The service's socket directory may
    /// lie in a folder other users can write to, so a socket folder inside
    /// it is readied and checked, with that directory, as the service's own
    /// socket directory is.
    ///
    /// # Errors
    /// Returns [`HolderError::InsecureSocketDir`] when it exists and other
    /// users can enter it, or the I/O error from creating it or, for one in
    /// the socket directory, of kind [`io::ErrorKind::PermissionDenied`]
    /// when it or its parent is not private to this user.
    pub fn prepare_sockets(&self) -> Result<(), HolderError> {
        if self.sockets == self.runtime_sockets
            && let Some(socket_dir) = self.sockets.parent()
        {
            prepare_socket_dir(socket_dir)?;
            return Ok(prepare_socket_dir(&self.sockets)?);
        }
        ensure_private_dir(&self.sockets)
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
    let hash = format!("{:016x}", fnv1a(name.as_ref().as_bytes()));
    if stem.len() > LOG_STEM_MAX {
        return hash;
    }
    format!(
        "{stem}{LOG_HASH_SEPARATOR}{}",
        &hash[..LOG_NAME_HASH_DIGITS]
    )
}

/// Creates `dir` owner-only when absent, and refuses an existing one other
/// users can enter rather than changing a possibly shared directory's mode.
///
/// # Errors
/// Returns [`HolderError::InsecureSocketDir`] when the directory is shared, or
/// the I/O error from creating or inspecting it.
pub fn ensure_private_dir(dir: &Path) -> Result<(), HolderError> {
    create_private_dir_all(dir)?;
    if fs::metadata(dir)?.permissions().mode() & GROUP_OTHER_BITS != 0 {
        return Err(HolderError::InsecureSocketDir(dir.to_path_buf()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        fs::{self, Permissions},
        os::unix::fs::PermissionsExt,
    };

    use muster_domain::value::PROCESS_NAME_MAX_CHARS;
    use muster_os::test_scratch::short_scratch;
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
    /// The scratch's stand-in for the service's socket directory.
    const RUNTIME_DIR: &str = "run";

    /// Holder dirs under `root`.
    fn dirs(root: &Path) -> HolderDirs {
        let sockets = root.join("sock");
        HolderDirs::builder()
            .sockets(sockets.clone())
            .state_sockets(sockets)
            .runtime_sockets(root.join(RUNTIME_DIR).join("holders"))
            .records(root.join("records"))
            .logs(root.join("logs"))
            .build()
    }

    /// A short state directory keeps its holder sockets, and a state
    /// directory too deep for one puts them in a folder of the socket
    /// directory named for it; both folders are swept either way.
    #[test]
    fn holder_sockets_fall_back_to_the_socket_dir_only_when_they_do_not_fit() {
        let scratch = short_scratch(SCRATCH_PREFIX);
        let socket_dir = scratch.path().join(RUNTIME_DIR);
        let short = scratch.path().join("state");
        let deep = scratch.path().join("d".repeat(DEEP_DIR_CHARS));

        let kept = HolderDirs::placed(&short, &socket_dir);
        let moved = HolderDirs::placed(&deep, &socket_dir);

        assert_eq!(kept.sockets(), kept.state_sockets());
        assert!(kept.sockets().starts_with(&short));
        assert_eq!(moved.sockets(), moved.runtime_sockets());
        assert_eq!(moved.sockets().parent(), Some(socket_dir.as_path()));
        assert_ne!(kept.runtime_sockets(), moved.runtime_sockets());
        let id = HolderId::generate().unwrap();
        assert!(fits_a_socket(&kept.socket_path(&id)));
        assert!(fits_a_socket(&moved.socket_path(&id)));
        assert!(!fits_a_socket(&moved.state_sockets().join(id.as_ref())));
        for dirs in [&kept, &moved] {
            assert_eq!(dirs.socket_folders(), vec![
                dirs.sockets(),
                other_folder(dirs)
            ]);
            assert!(dirs.is_socket(&other_folder(dirs).join("x.sock")));
        }
    }

    /// The folder of `dirs`' two that new holders do not bind in.
    fn other_folder(dirs: &HolderDirs) -> &PathBuf {
        if dirs.sockets() == dirs.state_sockets() {
            dirs.runtime_sockets()
        } else {
            dirs.state_sockets()
        }
    }

    /// A runtime sockets folder is made private with its parent, and one
    /// whose parent other users can enter is refused, as the service's own
    /// socket directory is.
    #[test]
    fn a_runtime_sockets_folder_is_readied_as_the_socket_dir_is() {
        let scratch = short_scratch(SCRATCH_PREFIX);
        let socket_dir = scratch.path().join(RUNTIME_DIR);
        let dirs = HolderDirs::placed(
            &scratch.path().join("d".repeat(DEEP_DIR_CHARS)),
            &socket_dir,
        );

        dirs.prepare_sockets().unwrap();

        for dir in [&socket_dir, dirs.sockets()] {
            let mode = fs::metadata(dir).unwrap().permissions().mode() & PERMISSION_BITS;
            assert_eq!(mode, PRIVATE_DIR_MODE, "{}", dir.display());
        }
        fs::set_permissions(&socket_dir, Permissions::from_mode(SHARED_DIR_MODE)).unwrap();
        assert!(matches!(dirs.prepare_sockets(), Err(HolderError::Io(error))
                if error.kind() == io::ErrorKind::PermissionDenied));
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
        let dirs = HolderDirs::placed(scratch.path(), &scratch.path().join(RUNTIME_DIR));
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
        let dirs = HolderDirs::placed(scratch.path(), &scratch.path().join(RUNTIME_DIR));
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
        let dirs = HolderDirs::placed(scratch.path(), &scratch.path().join(RUNTIME_DIR));
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
