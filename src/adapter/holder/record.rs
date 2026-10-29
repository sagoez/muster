use std::{
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
    time::SystemTime,
};

use atomic_write_file::AtomicWriteFile;
use getset::{CopyGetters, Getters};
use serde::{Deserialize, Serialize};
use typed_builder::TypedBuilder;

use super::{error::HolderError, id::HolderId};
use crate::{
    adapter::private_dir::create_private_dir_all,
    domain::{
        process::StopPolicy,
        pty::SpawnHistory,
        value::{CommandLine, ProcessName, ProjectRoot, StartToken},
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
    holder_pid: u32,
    /// The supervised child's id.
    #[getset(get_copy = "pub")]
    child_pid: u32,
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
    history: SpawnHistory,
}

impl HolderRecord {
    /// Writes this record atomically into `dir`, creating the directory
    /// owner-only.
    ///
    /// # Errors
    /// Returns a [`HolderError`] when encoding or the write fails.
    pub fn save(&self, dir: &Path) -> Result<(), HolderError> {
        create_private_dir_all(dir)?;
        let mut file = AtomicWriteFile::open(self.path_in(dir))?;
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
        .map(|path| {
            let record = fs::read(&path)
                .ok()
                .and_then(|bytes| serde_json::from_slice(&bytes).ok());
            (path, record)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::{os::unix::fs::PermissionsExt, time::Duration};

    use tempfile::TempDir;

    use super::*;
    use crate::domain::{process::StopSignal, value::PositiveDuration};

    /// Mode bits a permission check compares.
    const PERMISSION_BITS: u32 = 0o777;
    /// The mode of a directory only its owner can enter.
    const OWNER_DIR: u32 = 0o700;
    /// Grace of the recorded stop policy.
    const STOP_GRACE: Duration = Duration::from_secs(7);

    fn record() -> HolderRecord {
        HolderRecord::builder()
            .id(HolderId::generate().unwrap())
            .project(ProjectRoot::try_new(PathBuf::from("/repo")).unwrap())
            .process(ProcessName::try_new("dev server").unwrap())
            .command(CommandLine::try_new("npm run dev").unwrap())
            .stop(
                StopPolicy::builder()
                    .signal(StopSignal::Interrupt)
                    .grace_period(PositiveDuration::try_new(STOP_GRACE).unwrap())
                    .build(),
            )
            .launch_environment(vec![("PATH".into(), "/usr/bin".into())])
            .socket(PathBuf::from("/tmp/h.sock"))
            .holder_pid(10)
            .child_pid(11)
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

    #[test]
    fn a_missing_directory_holds_no_records() {
        let scratch = TempDir::new().unwrap();
        assert!(HolderRecord::load_all(&scratch.path().join("absent")).is_empty());
    }
}
