use std::{
    ffi::OsString,
    fs, io,
    os::unix::fs::OpenOptionsExt as _,
    path::{Path, PathBuf},
    time::SystemTime,
};

use atomic_write_file::{AtomicWriteFile, unix::OpenOptionsExt as _};
use getset::{CopyGetters, Getters};
use serde::{Deserialize, Serialize};
use typed_builder::TypedBuilder;

use super::{config::PRIVATE_FILE_MODE, error::HolderError, id::HolderId};
use crate::{
    adapter::private_dir::create_private_dir_all,
    domain::{
        process::StopPolicy,
        pty::SpawnHistory,
        value::{CommandLine, Pid, ProcessName, ProjectRoot, StartToken},
    },
};

/// File extension of a persisted holder record.
const RECORD_EXTENSION: &str = "json";

/// What a running holder persists so a later service can find and adopt it,
/// and stop it even once its process is no longer in the project.
#[derive(
    Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Getters, CopyGetters, TypedBuilder,
)]
#[serde(rename_all = "kebab-case")]
pub struct HolderRecord {
    /// The holder's identity, also its record's file stem.
    #[getset(get = "pub")]
    id: HolderId,
    /// The folder of the project the process belongs to.
    #[getset(get = "pub")]
    project: ProjectRoot,
    /// The process's name within its project.
    #[getset(get = "pub")]
    process: ProcessName,
    /// The command the child runs.
    #[getset(get = "pub")]
    command: CommandLine,
    /// How the process stops.
    #[getset(get_copy = "pub")]
    #[serde(with = "crate::adapter::wire::form")]
    stop: StopPolicy,
    /// The environment of the command that launched the child, which its
    /// restarts are launched with too.
    #[getset(get = "pub")]
    #[builder(default)]
    launch_environment: Vec<(OsString, OsString)>,
    /// Socket the holder serves.
    #[getset(get = "pub")]
    socket: PathBuf,
    /// The holder process's id.
    #[getset(get_copy = "pub")]
    holder_pid: Pid,
    /// The supervised child's id.
    #[getset(get_copy = "pub")]
    child_pid: Pid,
    /// The child's start token when the platform reports one, so a later
    /// process reusing `child_pid` is never taken for the child.
    #[getset(get_copy = "pub")]
    child_start_token: Option<StartToken>,
    /// The child's output log.
    #[getset(get = "pub")]
    log: PathBuf,
    /// When the child started, so an adopted process keeps its uptime.
    #[getset(get_copy = "pub")]
    child_started_at: SystemTime,
    /// What the child's earlier runs left it, so an adopting service keeps
    /// the restart count and last exit.
    #[getset(get = "pub")]
    #[builder(default)]
    #[serde(with = "crate::adapter::wire::form")]
    history: SpawnHistory,
}

impl HolderRecord {
    /// Writes this record atomically into `dir` as an owner-only file, since
    /// it carries the child's environment, creating the directory owner-only.
    ///
    /// # Errors
    /// Returns a [`HolderError`] when encoding or the write fails.
    pub fn save(&self, dir: &Path) -> Result<(), HolderError> {
        create_private_dir_all(dir)?;
        let mut file = AtomicWriteFile::options()
            .mode(PRIVATE_FILE_MODE)
            .preserve_mode(false)
            .open(self.path_in(dir))?;
        serde_json::to_writer(&mut file, self)?;
        file.commit()?;
        Ok(())
    }

    /// Reads every record in `dir`, skipping files that are unreadable or do
    /// not parse. A missing directory holds no records.
    pub fn load_all(dir: &Path) -> Vec<Self> {
        read_records(dir)
            .into_iter()
            .filter_map(|(_, record)| record)
            .collect()
    }

    /// The record files in `dir` that cannot be read or parsed, such as one a
    /// newer or older holder wrote in another shape.
    pub fn unreadable(dir: &Path) -> Vec<PathBuf> {
        read_records(dir)
            .into_iter()
            .filter(|(_, record)| record.is_none())
            .map(|(path, _)| path)
            .collect()
    }

    /// Deletes this record from `dir`; an already-absent record is not an error.
    ///
    /// # Errors
    /// Returns an [`io::Error`] when the file exists but cannot be removed.
    pub fn remove(&self, dir: &Path) -> io::Result<()> {
        match fs::remove_file(self.path_in(dir)) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
            _ => Ok(()),
        }
    }

    /// This record's file path inside `dir`.
    fn path_in(&self, dir: &Path) -> PathBuf {
        dir.join(self.id.as_ref()).with_extension(RECORD_EXTENSION)
    }
}

/// Every record file in `dir` with its parsed record, or `None` when it cannot
/// be read or parsed. A missing directory holds no records.
fn read_records(dir: &Path) -> Vec<(PathBuf, Option<HolderRecord>)> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == RECORD_EXTENSION))
        .filter_map(|path| read_record(&path).map(|record| (path, record)))
        .collect()
}

/// The record file at `path`, parsed, or `None` inside when it cannot be read
/// or parsed. A file removed since it was listed, as a finishing holder
/// removes its own, is absent: none at all.
fn read_record(path: &Path) -> Option<Option<HolderRecord>> {
    match fs::read(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        read => Some(
            read.ok()
                .and_then(|bytes| serde_json::from_slice(&bytes).ok()),
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::{os::unix::fs::PermissionsExt, time::Duration};

    use tempfile::TempDir;

    use super::*;
    use crate::domain::{process::StopSignal, value::StopGrace};

    /// Mode bits a permission check compares.
    const PERMISSION_BITS: u32 = 0o777;
    /// The mode of a directory only its owner can enter.
    const OWNER_DIR: u32 = 0o700;
    /// A file mode other users can read.
    const SHARED_FILE: u32 = 0o644;
    /// Grace of the recorded stop policy.
    const STOP_GRACE: Duration = Duration::from_secs(7);

    /// A record with every field set.
    fn record() -> HolderRecord {
        HolderRecord::builder()
            .id(HolderId::generate().unwrap())
            .project(ProjectRoot::try_new(PathBuf::from("/repo")).unwrap())
            .process(ProcessName::try_new("dev server").unwrap())
            .command(CommandLine::try_new("npm run dev").unwrap())
            .stop(
                StopPolicy::builder()
                    .signal(StopSignal::Interrupt)
                    .grace_period(StopGrace::try_new(STOP_GRACE).unwrap())
                    .build(),
            )
            .launch_environment(vec![("PATH".into(), "/usr/bin".into())])
            .socket(PathBuf::from("/tmp/h.sock"))
            .holder_pid(Pid::try_new(10).unwrap())
            .child_pid(Pid::try_new(11).unwrap())
            .child_start_token(Some(StartToken::try_new(12).unwrap()))
            .log(PathBuf::from("/state/logs/dev.log"))
            .child_started_at(SystemTime::UNIX_EPOCH)
            .build()
    }

    #[test]
    fn record_save_load_round_trips_and_load_all_skips_corrupt_files() {
        let scratch = TempDir::new().unwrap();
        let dir = scratch.path().join("records");
        let saved = record();
        saved.save(&dir).unwrap();
        fs::write(dir.join("corrupt.json"), b"{not json").unwrap();
        fs::write(dir.join("notes.txt"), b"ignored").unwrap();

        assert_eq!(HolderRecord::load_all(&dir), vec![saved.clone()]);
        assert_eq!(HolderRecord::unreadable(&dir), vec![
            dir.join("corrupt.json")
        ]);

        saved.remove(&dir).unwrap();
        saved.remove(&dir).unwrap();
        assert!(HolderRecord::load_all(&dir).is_empty());
    }

    /// The records directory a save creates is its owner's alone.
    #[test]
    fn the_records_directory_is_private() {
        let scratch = TempDir::new().unwrap();
        let dir = scratch.path().join("holders").join("records");

        record().save(&dir).unwrap();

        for created in [dir.as_path(), dir.parent().unwrap()] {
            let mode = fs::metadata(created).unwrap().permissions().mode() & PERMISSION_BITS;
            assert_eq!(mode, OWNER_DIR, "{}", created.display());
        }
    }

    /// A saved record is its owner's alone, even over one another user could
    /// read.
    #[test]
    fn a_saved_record_is_private() {
        let scratch = TempDir::new().unwrap();
        let dir = scratch.path().join("records");
        let saved = record();
        saved.save(&dir).unwrap();
        let path = saved.path_in(&dir);
        let mode_of =
            |path: &Path| fs::metadata(path).unwrap().permissions().mode() & PERMISSION_BITS;
        assert_eq!(mode_of(&path), PRIVATE_FILE_MODE);

        fs::set_permissions(&path, fs::Permissions::from_mode(SHARED_FILE)).unwrap();
        saved.save(&dir).unwrap();

        assert_eq!(mode_of(&path), PRIVATE_FILE_MODE);
    }

    #[test]
    fn a_missing_directory_holds_no_records() {
        let scratch = TempDir::new().unwrap();
        assert!(HolderRecord::load_all(&scratch.path().join("absent")).is_empty());
    }

    /// A record removed between listing and reading, as a finishing holder
    /// removes its own, is absent rather than unreadable; one that does not
    /// parse is unreadable.
    #[test]
    fn a_record_gone_before_it_is_read_is_absent() {
        let scratch = TempDir::new().unwrap();
        let dir = scratch.path().join("records");
        let saved = record();
        saved.save(&dir).unwrap();
        let path = saved.path_in(&dir);

        assert!(matches!(read_record(&path), Some(Some(_))));
        fs::write(&path, b"not json").unwrap();
        assert_eq!(read_record(&path), Some(None));
        fs::remove_file(&path).unwrap();
        assert_eq!(read_record(&path), None);
    }
}
