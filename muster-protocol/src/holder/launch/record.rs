use std::{
    ffi::OsString,
    fs, io,
    os::unix::fs::OpenOptionsExt as _,
    path::{Path, PathBuf},
    time::SystemTime,
};

use atomic_write_file::{AtomicWriteFile, unix::OpenOptionsExt as _};
use getset::{CopyGetters, Getters};
use muster_domain::{
    process::StopPolicy,
    pty::SpawnHistory,
    value::{BootId, CommandLine, Pid, ProcessName, ProjectRoot, StartToken},
};
use muster_os::{bounded_read::read_bounded_unlinked, private_dir::create_private_dir_all};
use serde::{Deserialize, Serialize};
use typed_builder::TypedBuilder;

use crate::holder::{
    error::HolderError,
    launch::{config::PRIVATE_FILE_MODE, id::HolderId, left_exit::LeftExit},
};

/// File extension of a persisted holder record.
const RECORD_EXTENSION: &str = "json";
/// File extension of the exit a holder that could not be reached leaves
/// beside its record.
const EXIT_EXTENSION: &str = "exit";
/// Largest record file read: past the JSON of the largest environment a
/// command can be launched with, whose bytes it spells out one number each.
const MAX_RECORD_BYTES: u64 = 32 * 1024 * 1024;
/// Largest exit file read: past any exit and cursor a holder writes.
const MAX_EXIT_BYTES: u64 = 1024;

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
    #[serde(with = "crate::wire::form")]
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
    /// The holder's start token when the platform reports one, so a later
    /// process reusing `holder_pid` is never taken for the holder. None in a
    /// record an earlier muster wrote.
    #[getset(get_copy = "pub")]
    #[builder(default)]
    holder_start_token: Option<StartToken>,
    /// The boot the holder started in, where the platform names one, since
    /// a start token may come round again in a later boot. None in a record
    /// an earlier muster wrote.
    #[getset(get = "pub")]
    #[builder(default)]
    boot_id: Option<BootId>,
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
    #[serde(with = "crate::wire::form")]
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

    /// Deletes this record from `dir`, with any exit left beside it; an
    /// already-absent record is not an error.
    ///
    /// # Errors
    /// Returns an [`io::Error`] when a file exists but cannot be removed.
    pub fn remove(&self, dir: &Path) -> io::Result<()> {
        remove_if_present(&self.exit_path_in(dir))?;
        remove_if_present(&self.path_in(dir))
    }

    /// Whether this record's file is in `dir`.
    pub fn is_saved_in(&self, dir: &Path) -> bool {
        fs::symlink_metadata(self.path_in(dir)).is_ok()
    }

    /// Saves this record in `dir` again, in case something removed it, then
    /// leaves `left` beside it as an owner-only file: the exit of a child
    /// whose holder could not be reached to have it collected, for the next
    /// service to report. The record comes first, so an exit is never seen
    /// without one.
    ///
    /// # Errors
    /// Returns a [`HolderError`] when encoding or a write fails.
    pub fn save_stranded(&self, dir: &Path, left: &LeftExit) -> Result<(), HolderError> {
        self.save(dir)?;
        let mut file = AtomicWriteFile::options()
            .mode(PRIVATE_FILE_MODE)
            .preserve_mode(false)
            .open(self.exit_path_in(dir))?;
        serde_json::to_writer(&mut file, left)?;
        file.commit()?;
        Ok(())
    }

    /// The exit a holder that could not be reached left beside this record
    /// in `dir`, when there is one that reads: a regular file, never a link
    /// or a FIFO, of at most [`MAX_EXIT_BYTES`].
    pub fn left_exit(&self, dir: &Path) -> Option<LeftExit> {
        let text = read_bounded_unlinked(&self.exit_path_in(dir), MAX_EXIT_BYTES).ok()?;
        serde_json::from_str(&text).ok()
    }

    /// Removes every exit left in `dir` beside no record, as one a holder
    /// left after its record was deleted, which nothing would report.
    /// Returns each that could not be removed, with why.
    pub fn sweep_orphan_exits(dir: &Path) -> Vec<(PathBuf, io::Error)> {
        let Ok(entries) = fs::read_dir(dir) else {
            return Vec::new();
        };
        entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == EXIT_EXTENSION))
            .filter(|path| fs::symlink_metadata(path.with_extension(RECORD_EXTENSION)).is_err())
            .filter_map(|path| remove_if_present(&path).err().map(|error| (path, error)))
            .collect()
    }

    /// This record's file path inside `dir`.
    fn path_in(&self, dir: &Path) -> PathBuf {
        dir.join(self.id.as_ref()).with_extension(RECORD_EXTENSION)
    }

    /// The path of the exit left beside this record inside `dir`.
    fn exit_path_in(&self, dir: &Path) -> PathBuf {
        dir.join(self.id.as_ref()).with_extension(EXIT_EXTENSION)
    }
}

/// Deletes the file at `path`; an absent one is not an error.
///
/// # Errors
/// Returns an [`io::Error`] when the file exists but cannot be removed.
fn remove_if_present(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
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
/// or parsed, or is not a regular file of at most [`MAX_RECORD_BYTES`]: a
/// link or a FIFO is never followed or waited on. A file removed since it
/// was listed, as a finishing holder removes its own, is absent: none at
/// all.
fn read_record(path: &Path) -> Option<Option<HolderRecord>> {
    match read_bounded_unlinked(path, MAX_RECORD_BYTES) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        read => Some(read.ok().and_then(|text| serde_json::from_str(&text).ok())),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        os::unix::fs::{PermissionsExt, symlink},
        time::Duration,
    };

    use muster_domain::{
        process::StopSignal,
        pty::ExitOutcome,
        value::{OutputCursor, StopGrace},
    };
    use muster_os::test_fifo::make_fifo;
    use tempfile::TempDir;

    use super::*;

    /// Where the left exit's run began.
    const RUN_START: u64 = 42;
    /// Where a test keeps the real exit file a link points to.
    const REAL_EXIT: &str = "real.exit";
    /// Where a test keeps the real record file a link points to.
    const REAL_RECORD: &str = "real.json";
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

    /// An exit left at the run start of the log.
    fn left() -> LeftExit {
        LeftExit::builder()
            .outcome(ExitOutcome::FAILED_UNKNOWN)
            .run_start(OutputCursor::new(RUN_START))
            .build()
    }

    /// An exit left beside a record reads back as it was, with its run's
    /// start, private, is not read as a record, and goes when the record
    /// does.
    #[test]
    fn an_exit_left_beside_a_record_reads_back_and_goes_with_it() {
        let scratch = TempDir::new().unwrap();
        let dir = scratch.path();
        let saved = record();
        saved.save(dir).unwrap();
        assert_eq!(saved.left_exit(dir), None);

        saved.save_stranded(dir, &left()).unwrap();

        assert_eq!(saved.left_exit(dir), Some(left()));
        let mode = fs::metadata(saved.exit_path_in(dir))
            .unwrap()
            .permissions()
            .mode()
            & PERMISSION_BITS;
        assert_eq!(mode, PRIVATE_FILE_MODE);
        assert_eq!(HolderRecord::load_all(dir), vec![saved.clone()]);
        assert!(HolderRecord::unreadable(dir).is_empty());
        saved.remove(dir).unwrap();
        assert_eq!(saved.left_exit(dir), None);
        assert!(HolderRecord::load_all(dir).is_empty());
    }

    /// A holder that strands its exit after something deleted its record
    /// saves the record again, so the exit is never left beside none.
    #[test]
    fn a_stranded_exit_brings_back_a_deleted_record() {
        let scratch = TempDir::new().unwrap();
        let dir = scratch.path();
        let saved = record();
        saved.save(dir).unwrap();
        assert!(saved.is_saved_in(dir));
        saved.remove(dir).unwrap();
        assert!(!saved.is_saved_in(dir));

        saved.save_stranded(dir, &left()).unwrap();

        assert_eq!(HolderRecord::load_all(dir), vec![saved.clone()]);
        assert_eq!(saved.left_exit(dir), Some(left()));
    }

    /// An exit file that is a FIFO, a link or past the cap is never read:
    /// no exit, and no wait on a FIFO with no writer.
    #[test]
    fn an_exit_file_that_is_not_a_small_regular_file_is_never_read() {
        let scratch = TempDir::new().unwrap();
        let dir = scratch.path();
        let saved = record();
        saved.save(dir).unwrap();
        let exit = saved.exit_path_in(dir);
        let real = dir.join(REAL_EXIT);
        fs::write(&real, serde_json::to_string(&left()).unwrap()).unwrap();

        make_fifo(&exit);
        assert_eq!(saved.left_exit(dir), None, "a FIFO");
        fs::remove_file(&exit).unwrap();
        symlink(&real, &exit).unwrap();
        assert_eq!(saved.left_exit(dir), None, "a link");
        fs::remove_file(&exit).unwrap();
        let mut oversized = serde_json::to_string(&left()).unwrap();
        oversized.push_str(&" ".repeat(usize::try_from(MAX_EXIT_BYTES).unwrap()));
        fs::write(&exit, oversized).unwrap();
        assert_eq!(saved.left_exit(dir), None, "past the cap");
    }

    /// A record file that is a FIFO or a link is unreadable, never waited on
    /// or followed.
    #[test]
    fn a_record_file_that_is_not_a_regular_file_is_unreadable() {
        let scratch = TempDir::new().unwrap();
        let dir = scratch.path().join("records");
        let saved = record();
        saved.save(&dir).unwrap();
        let path = saved.path_in(&dir);
        let real = scratch.path().join(REAL_RECORD);
        fs::rename(&path, &real).unwrap();

        make_fifo(&path);
        assert_eq!(read_record(&path), Some(None), "a FIFO");
        fs::remove_file(&path).unwrap();
        symlink(&real, &path).unwrap();
        assert_eq!(read_record(&path), Some(None), "a link");
    }

    /// An exit beside no record is swept, and one beside its record is
    /// kept.
    #[test]
    fn an_exit_beside_no_record_is_swept() {
        let scratch = TempDir::new().unwrap();
        let dir = scratch.path();
        let kept = record();
        kept.save_stranded(dir, &left()).unwrap();
        let orphan = record();
        orphan.save_stranded(dir, &left()).unwrap();
        fs::remove_file(orphan.path_in(dir)).unwrap();

        assert!(HolderRecord::sweep_orphan_exits(dir).is_empty());

        assert_eq!(orphan.left_exit(dir), None);
        assert_eq!(kept.left_exit(dir), Some(left()));
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
