use std::{
    fs, io,
    path::{Path, PathBuf},
};

use getset::Getters;
use muster_domain::value::{ProcessName, ProjectRoot};
use muster_os::private_dir::{
    create_private_dir_all, is_owned_private_dir, owner_of, tmp_socket_dir,
};
use typed_builder::TypedBuilder;

use crate::{
    holder::{error::HolderError, launch::id::HolderId},
    key::{hash_key, path_key},
};

/// File extension of a holder socket.
const SOCKET_EXTENSION: &str = "sock";
/// File extension of a holder's output log.
const LOG_EXTENSION: &str = "log";
/// Directory under the state directory holding every holder's records.
const HOLDERS_DIR: &str = "holders";
/// Directory of holder records.
const RECORDS_DIR: &str = "records";
/// Directory of each process's output log, kept outside the holders' own
/// files because a log outlives every holder that wrote it.
const LOGS_DIR: &str = "logs";
/// Directory under the holders' directory where an earlier muster kept one
/// log per holder.
const LEGACY_LOGS_DIR: &str = "logs";
/// Directory under the holders' directory where an earlier muster bound
/// holder sockets and handed over their specs.
const LEGACY_SOCKETS_DIR: &str = "sock";
/// Punctuation a process name keeps in its log file name.
const LOG_NAME_KEPT: &[u8] = b"-_ ";
/// Longest log file stem kept as written, well inside the 255-byte limit.
const LOG_STEM_MAX: usize = 200;
/// Hex digits of the name's hash a log file name ends with.
const LOG_NAME_HASH_DIGITS: usize = 8;
/// Between a log file name's encoded process name and its hash.
const LOG_HASH_SEPARATOR: char = '.';
/// Folder of a test's holders standing in for `/tmp/muster-<uid>`.
#[cfg(any(test, feature = "testing"))]
const SCRATCH_SOCKET_ROOT: &str = "socket-root";
/// The sockets' folder in that stand-in.
#[cfg(any(test, feature = "testing"))]
const SCRATCH_SOCKETS_DIR: &str = "sock";

/// Where holders keep their sockets, records and logs.
#[derive(Debug, Clone, Getters, TypedBuilder)]
pub struct HolderDirs {
    /// Directory of holder sockets and the handover files beside them, in a
    /// root private to the user, named for the state directory.
    #[getset(get = "pub")]
    sockets: PathBuf,
    /// Directory in the state directory where an earlier muster bound holder
    /// sockets: holders it launched are still adopted from there, and what
    /// they leave is swept from it.
    #[getset(get = "pub")]
    legacy_sockets: PathBuf,
    /// Directory of holder records.
    #[getset(get = "pub")]
    records: PathBuf,
    /// Directory of child output logs.
    #[getset(get = "pub")]
    logs: PathBuf,
}

impl HolderDirs {
    /// The holder directories a service owning `state_dir` keeps: records
    /// and logs in the state directory, and sockets in `/tmp/muster-<uid>`, a
    /// short path that always fits a Unix socket address and that no logout
    /// removes.
    pub fn placed(state_dir: &Path) -> Self {
        Self::placed_in(state_dir, &tmp_socket_dir())
    }

    /// As [`Self::placed`], with the sockets' folder in `socket_root`, named
    /// for the state directory with its links resolved, so two state
    /// directories never share one and two spellings of one always do.
    pub fn placed_in(state_dir: &Path, socket_root: &Path) -> Self {
        let holders = state_dir.join(HOLDERS_DIR);
        let named = fs::canonicalize(state_dir).unwrap_or_else(|_| state_dir.to_path_buf());
        Self::builder()
            .sockets(socket_root.join(path_key(&named)))
            .legacy_sockets(holders.join(LEGACY_SOCKETS_DIR))
            .records(holders.join(RECORDS_DIR))
            .logs(state_dir.join(LOGS_DIR))
            .build()
    }

    /// Holder dirs for a test: sockets in `holders/socket-root/sock`,
    /// records and the legacy sockets' folder under `holders`, and logs in
    /// `logs`.
    #[cfg(any(test, feature = "testing"))]
    pub fn scratch(holders: &Path, logs: PathBuf) -> Self {
        Self::builder()
            .sockets(holders.join(SCRATCH_SOCKET_ROOT).join(SCRATCH_SOCKETS_DIR))
            .legacy_sockets(holders.join(HOLDERS_DIR).join(LEGACY_SOCKETS_DIR))
            .records(holders.join(RECORDS_DIR))
            .logs(logs)
            .build()
    }

    /// Refuses the sockets' folder unless it and its parent are private, as
    /// [`check_socket_folder`] does.
    ///
    /// # Errors
    /// As [`check_socket_folder`].
    pub fn check_sockets(&self) -> Result<(), HolderError> {
        check_socket_folder(&self.sockets)
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

    /// Whether `path` names a holder socket, as [`Self::socket_path`] makes
    /// them.
    pub fn is_socket(&self, path: &Path) -> bool {
        path.parent() == Some(self.sockets.as_path())
            && path.extension().is_some_and(|ext| ext == SOCKET_EXTENSION)
    }

    /// Whether `path` names a holder socket an earlier muster bound in the
    /// legacy sockets' folder.
    pub fn is_legacy_socket(&self, path: &Path) -> bool {
        path.parent() == Some(self.legacy_sockets.as_path())
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

    /// Creates the sockets' folder owner-only when absent, as
    /// [`ensure_socket_folder`] does, so a holder is only ever handed a
    /// private one.
    ///
    /// # Errors
    /// As [`ensure_socket_folder`].
    pub fn prepare_sockets(&self) -> Result<(), HolderError> {
        ensure_socket_folder(&self.sockets)
    }

    /// Removes the legacy sockets' folder once nothing is left in it. An
    /// absent folder is not an error; one still holding a file is left.
    ///
    /// # Errors
    /// Returns the I/O error from removing it, of kind
    /// [`io::ErrorKind::DirectoryNotEmpty`] when something is left in it.
    pub fn remove_legacy_sockets(&self) -> io::Result<()> {
        match fs::remove_dir(&self.legacy_sockets) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            removed => removed,
        }
    }
}

/// Creates the socket folder `folder` and the folder holding it owner-only
/// when absent, the holding one first, since it lies in `/tmp`, which every
/// user can write to, and refuses either unless private, as
/// [`check_socket_folder`] does.
///
/// # Errors
/// Returns [`HolderError::InsecureSocketDir`] naming a directory that is not
/// private to this user, or the I/O error from creating one.
pub fn ensure_socket_folder(folder: &Path) -> Result<(), HolderError> {
    if let Some(root) = holding_folder(folder) {
        ensure_private_dir(root)?;
    }
    ensure_private_dir(folder)
}

/// Refuses the socket folder `folder` unless it and the folder holding it
/// are real directories this user owns with mode 0700.
///
/// # Errors
/// Returns [`HolderError::InsecureSocketDir`] naming the directory that is
/// not private, or the I/O error from inspecting one, of kind
/// [`io::ErrorKind::NotFound`] when it is absent.
pub fn check_socket_folder(folder: &Path) -> Result<(), HolderError> {
    if let Some(root) = holding_folder(folder) {
        check_private_dir(root)?;
    }
    check_private_dir(folder)
}

/// The folder holding the socket folder `folder`, when it has one.
fn holding_folder(folder: &Path) -> Option<&Path> {
    folder.parent().filter(|root| !root.as_os_str().is_empty())
}

/// The file named for `id` with `extension` inside `dir`.
fn file_in(dir: &Path, id: &HolderId, extension: &str) -> PathBuf {
    dir.join(id.as_ref()).with_extension(extension)
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
/// with mode 0700: the rule the service's socket directory follows, so a
/// holder trusts its socket's folder only as far as the service does.
///
/// # Errors
/// Returns [`HolderError::InsecureSocketDir`] naming it and its owner when
/// it is not private, or the I/O error from inspecting it, of kind
/// [`io::ErrorKind::NotFound`] when it is absent.
pub fn check_private_dir(dir: &Path) -> Result<(), HolderError> {
    if is_owned_private_dir(dir)? {
        return Ok(());
    }
    Err(HolderError::InsecureSocketDir {
        dir: dir.to_path_buf(),
        owner: owner_of(dir),
    })
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
    use crate::ipc::fits_a_socket;

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
    const SOCKET_ROOT: &str = "tmp";
    /// The root `/tmp/muster-<uid>` lies in.
    const TMP: &str = "/tmp";
    /// The name of `/tmp/muster-<uid>` before the uid.
    const TMP_SOCKET_PREFIX: &str = "muster-";
    /// A link a test plants in place of a folder.
    const PLANTED_LINK: &str = "planted";
    /// A socket folder another user made.
    const SQUATTED: &str = "/tmp/muster-1000";
    /// The uid of the user who made it.
    const SQUATTER_UID: u32 = 1001;
    /// What a refused folder's error says to have done to it.
    const REMEDY_OWNER: &str = "have its owner or an administrator remove it";
    /// What a refused folder's error says clears it otherwise.
    const REMEDY_REBOOT: &str = "reboot";

    /// Holder dirs under `root`.
    fn dirs(root: &Path) -> HolderDirs {
        HolderDirs::scratch(root, root.join("logs"))
    }

    /// Holder dirs of a state directory in `scratch`, with the socket root
    /// in it too.
    fn scratch_dirs(scratch: &Path) -> HolderDirs {
        HolderDirs::placed_in(&scratch.join("state"), &scratch.join(SOCKET_ROOT))
    }

    /// Holder sockets live in `/tmp/muster-<uid>`, in a folder named for the
    /// state directory, never in the state directory, which keeps the
    /// records and logs; a socket there fits however deep the state
    /// directory is, and two state directories get two folders.
    #[test]
    fn holder_sockets_live_in_tmp_named_for_the_state_dir() {
        let deep = Path::new(TMP).join("d".repeat(DEEP_DIR_CHARS));
        let other = Path::new(TMP).join("state");

        let dirs = HolderDirs::placed(&deep);

        let root = dirs.sockets().parent().unwrap();
        assert_eq!(root, tmp_socket_dir());
        assert_eq!(root.parent(), Some(Path::new(TMP)));
        assert!(
            root.ends_with(format!("{TMP_SOCKET_PREFIX}{}", geteuid().as_raw())),
            "{}",
            root.display()
        );
        assert!(!dirs.sockets().starts_with(&deep));
        assert!(dirs.records().starts_with(&deep) && dirs.logs().starts_with(&deep));
        assert_ne!(dirs.sockets(), HolderDirs::placed(&other).sockets());
        let socket = dirs.socket_path(&HolderId::generate().unwrap());
        assert!(fits_a_socket(&socket));
        assert!(dirs.is_socket(&socket));
    }

    /// The sockets' folder is made private with its parent, and one whose
    /// parent other users can enter is refused, naming that parent.
    #[test]
    fn the_sockets_folder_is_readied_with_its_parent() {
        let scratch = short_scratch(SCRATCH_PREFIX);
        let dirs = scratch_dirs(scratch.path());
        let root = scratch.path().join(SOCKET_ROOT);

        dirs.prepare_sockets().unwrap();

        for dir in [&root, dirs.sockets()] {
            let mode = fs::metadata(dir).unwrap().permissions().mode() & PERMISSION_BITS;
            assert_eq!(mode, PRIVATE_DIR_MODE, "{}", dir.display());
        }
        dirs.check_sockets().unwrap();
        fs::set_permissions(&root, Permissions::from_mode(SHARED_DIR_MODE)).unwrap();
        for refused in [dirs.prepare_sockets(), dirs.check_sockets()] {
            assert!(
                matches!(&refused, Err(HolderError::InsecureSocketDir { dir, owner })
                    if *dir == root && *owner == Some(geteuid().as_raw())),
                "{refused:?}"
            );
        }
    }

    /// A refused socket folder's error names the folder, who owns it and
    /// what clears it, since its fixed path cannot be moved.
    #[test]
    fn a_refused_socket_folder_names_its_owner_and_the_remedy() {
        let error = HolderError::InsecureSocketDir {
            dir: PathBuf::from(SQUATTED),
            owner: Some(SQUATTER_UID),
        }
        .to_string();

        for needed in [
            SQUATTED,
            &SQUATTER_UID.to_string(),
            REMEDY_OWNER,
            REMEDY_REBOOT,
        ] {
            assert!(error.contains(needed), "{error}");
        }
    }

    /// Two spellings of one state directory, one through a link, name the
    /// same sockets' folder, so neither leaves the other's sockets unswept.
    #[test]
    fn a_linked_state_dir_shares_its_sockets_folder() {
        let scratch = short_scratch(SCRATCH_PREFIX);
        let state = scratch.path().join("state");
        fs::create_dir(&state).unwrap();
        let link = scratch.path().join(PLANTED_LINK);
        symlink(&state, &link).unwrap();
        let root = scratch.path().join(SOCKET_ROOT);

        assert_eq!(
            HolderDirs::placed_in(&link, &root).sockets(),
            HolderDirs::placed_in(&state, &root).sockets()
        );
    }

    /// The legacy sockets' folder is removed once empty, left while it
    /// holds a file, and its absence is no error.
    #[test]
    fn the_legacy_sockets_folder_is_removed_only_once_empty() {
        let scratch = short_scratch(SCRATCH_PREFIX);
        let dirs = dirs(scratch.path());
        dirs.remove_legacy_sockets().unwrap();
        fs::create_dir_all(dirs.legacy_sockets()).unwrap();
        let left = dirs.legacy_sockets().join("left.json");
        fs::write(&left, b"").unwrap();

        assert!(dirs.remove_legacy_sockets().is_err());
        assert!(left.exists());
        fs::remove_file(&left).unwrap();
        dirs.remove_legacy_sockets().unwrap();
        assert!(!dirs.legacy_sockets().exists());
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
            Err(HolderError::InsecureSocketDir { .. })
        ));
        assert!(matches!(
            check_private_dir(&link),
            Err(HolderError::InsecureSocketDir { .. })
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
            Err(HolderError::InsecureSocketDir { .. })
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
        let dirs = HolderDirs::placed_in(scratch.path(), &scratch.path().join(SOCKET_ROOT));
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
        let dirs = HolderDirs::placed_in(scratch.path(), &scratch.path().join(SOCKET_ROOT));
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
        let dirs = HolderDirs::placed_in(scratch.path(), &scratch.path().join(SOCKET_ROOT));
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
