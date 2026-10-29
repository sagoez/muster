use std::{
    fs, io,
    path::{Path, PathBuf},
};

use getset::Getters;
use typed_builder::TypedBuilder;

use super::{error::HolderError, id::HolderId, server::ensure_private_dir};
use crate::domain::value::{ProcessName, ProjectRoot};

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
/// FNV-1a's 64-bit offset basis.
const FNV_OFFSET_BASIS: u64 = 0xCBF2_9CE4_8422_2325;
/// FNV-1a's 64-bit prime.
const FNV_PRIME: u64 = 0x0000_0100_0000_01B3;

/// Where holders keep their sockets, records and logs.
#[derive(Debug, Clone, Getters, TypedBuilder)]
pub struct HolderDirs {
    /// Directory of holder sockets; private to its owner.
    #[getset(get = "pub")]
    sockets: PathBuf,
    /// Directory of holder records.
    #[getset(get = "pub")]
    records: PathBuf,
    /// Directory of child output logs.
    #[getset(get = "pub")]
    logs: PathBuf,
}

impl HolderDirs {
    /// The holder directories a service owning `state_dir` keeps, which
    /// every later service of that state directory adopts from.
    pub fn under(state_dir: &Path) -> Self {
        let holders = state_dir.join(HOLDERS_DIR);
        Self::builder()
            .sockets(holders.join(SOCKETS_DIR))
            .records(holders.join(RECORDS_DIR))
            .logs(state_dir.join(LOGS_DIR))
            .build()
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

    /// The log of the process `name` of the project at `project`: one file
    /// per process that every run appends to, so it can be read after the
    /// process exits. The project's folder is named by a stable hash of its
    /// root, which is already canonical, so every alias of it shares the log.
    pub fn process_log(&self, project: &ProjectRoot, name: &ProcessName) -> PathBuf {
        self.logs
            .join(format!(
                "{:016x}",
                fnv1a(project.as_ref().as_os_str().as_encoded_bytes())
            ))
            .join(format!("{}.{LOG_EXTENSION}", log_file_stem(name)))
    }

    /// Creates the socket directory owner-only when absent, so a holder is
    /// only ever handed a private one.
    ///
    /// # Errors
    /// Returns [`HolderError::InsecureSocketDir`] when it exists and other
    /// users can enter it, or the I/O error from creating it.
    pub fn prepare_sockets(&self) -> Result<(), HolderError> {
        ensure_private_dir(&self.sockets)
    }
}

/// The file named for `id` with `extension` inside `dir`.
fn file_in(dir: &Path, id: &HolderId, extension: &str) -> PathBuf {
    dir.join(id.as_ref()).with_extension(extension)
}

/// A file name stem for `name` that is safe on every supported file system:
/// ASCII letters, digits, `-`, `_` and spaces are kept, and every other byte
/// is percent-encoded, a leading dot included, so no name can climb out of
/// its folder or hide. A name too long for a file name is replaced by its
/// hash.
fn log_file_stem(name: &ProcessName) -> String {
    let mut stem = String::new();
    for byte in name.as_ref().bytes() {
        if byte.is_ascii_alphanumeric() || LOG_NAME_KEPT.contains(&byte) {
            stem.push(char::from(byte));
        } else {
            stem.push_str(&format!("%{byte:02X}"));
        }
    }
    if stem.len() > LOG_STEM_MAX {
        return format!("{:016x}", fnv1a(name.as_ref().as_bytes()));
    }
    stem
}

/// The 64-bit FNV-1a hash of `bytes`: stable across builds and platforms,
/// unlike the standard library's hasher.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(FNV_OFFSET_BASIS, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(FNV_PRIME)
    })
}

#[cfg(test)]
mod tests {
    use std::{
        fs::{self, Permissions},
        os::unix::fs::PermissionsExt,
    };

    use tempfile::TempDir;

    use super::*;

    /// A directory mode other users can enter.
    const SHARED_DIR_MODE: u32 = 0o755;
    /// The mode a created socket directory gets.
    const PRIVATE_DIR_MODE: u32 = 0o700;
    /// Mode bits a permission check compares.
    const PERMISSION_BITS: u32 = 0o777;

    fn dirs(root: &Path) -> HolderDirs {
        HolderDirs::builder()
            .sockets(root.join("sock"))
            .records(root.join("records"))
            .logs(root.join("logs"))
            .build()
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
        let dirs = HolderDirs::under(scratch.path());
        let project = root(scratch.path().join("shop"));

        let log = dirs.process_log(&project, &name("dev server"));

        assert_eq!(log.file_name().unwrap(), "dev server.log");
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
        assert_eq!(
            log_file_stem(&name("../etc/passwd")),
            "%2E%2E%2Fetc%2Fpasswd"
        );
        assert_eq!(log_file_stem(&name("café")), "caf%C3%A9");
        assert_eq!(log_file_stem(&name("dev_server-2")), "dev_server-2");
        let long = name(&"x".repeat(LOG_STEM_MAX + 1));
        assert_eq!(log_file_stem(&long).len(), 16);
        assert_ne!(log_file_stem(&long), log_file_stem(&name(&"y".repeat(300))));
    }

    /// The legacy per-holder log directory is removed whole, and removing it
    /// again when it is gone is no error; the current logs are untouched.
    #[test]
    fn the_legacy_log_directory_is_removed_once() {
        let scratch = TempDir::new().unwrap();
        let dirs = HolderDirs::under(scratch.path());
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

    /// The hash is the published FNV-1a, so log folders survive rebuilds.
    #[test]
    fn the_folder_hash_is_stable_fnv1a() {
        assert_eq!(fnv1a(b""), FNV_OFFSET_BASIS);
        assert_eq!(fnv1a(b"a"), 0xAF63_DC4C_8601_EC8C);
    }
}
