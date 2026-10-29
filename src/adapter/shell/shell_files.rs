use std::{
    fs,
    io::{self, Write as _},
    os::unix::fs::OpenOptionsExt as _,
    path::{Path, PathBuf},
};

use atomic_write_file::{AtomicWriteFile, unix::OpenOptionsExt as _};

use crate::{
    adapter::{
        private_dir::create_private_dir_all,
        process_identity::LocalProcessIdentity,
        signal_target::SignalTarget,
        wire::{ShellRecordWire, WireForm},
    },
    domain::{
        occupancy::{ReportStamp, ShellId, ShellRecord, ShellRegistryError},
        port::ShellRegistry,
        value::Pid,
    },
};

/// The folder under the state directory that holds every shell's files.
const SHELLS_DIR: &str = "shells";
/// What ends a shell's record file, after its pid.
const RECORD_SUFFIX: &str = ".json";
/// What ends a note file, after the shell's pid and the report's stamp.
const NOTE_SUFFIX: &str = ".note";
/// Between the parts of a note file's name.
const NOTE_SEPARATOR: char = '-';
/// Digits a note file's load and seq are each padded to, so the files of
/// one load sort by seq.
const STAMP_WIDTH: usize = 20;
/// Most note files one shell may have waiting; a delivery beyond it is
/// dropped whole, so a hook that stopped printing cannot fill the disk.
const MAX_NOTE_FILES: usize = 16;
/// Mode of every record and note file: its owner's alone, since a note can
/// name what a process runs.
const PRIVATE_FILE_MODE: u32 = 0o600;

/// The shells running muster's hook, kept as files under
/// `<state dir>/shells/`: `<pid>.json` holds a shell's last report, and
/// `<pid>-<start>-<load>-<seq>.note` the lines its hook prints at its next
/// prompt, named by the whole shell so a later shell on a reused pid never
/// has its notes taken for an earlier one's.
#[derive(Debug, Clone)]
pub struct ShellFiles {
    /// The folder every shell's files live in.
    dir: PathBuf,
}

impl ShellFiles {
    /// The shell files under `state_dir`.
    pub fn under(state_dir: &Path) -> Self {
        Self {
            dir: state_dir.join(SHELLS_DIR),
        }
    }

    /// The record file of the shell running as `pid`.
    fn record_path(&self, pid: Pid) -> PathBuf {
        self.dir.join(format!("{pid}{RECORD_SUFFIX}"))
    }

    /// The note file of `shell` for the report stamped `stamp`.
    fn note_path(&self, shell: &ShellId, stamp: ReportStamp) -> PathBuf {
        self.dir.join(format!(
            "{}{:0width$}{NOTE_SEPARATOR}{:0width$}{NOTE_SUFFIX}",
            note_prefix(shell),
            stamp.load().into_inner(),
            stamp.seq().into_inner(),
            width = STAMP_WIDTH
        ))
    }

    /// The record stored for whichever shell ran as `pid`, none when absent
    /// or unreadable.
    fn stored(&self, pid: Pid) -> Option<ShellRecord> {
        read_record(&self.record_path(pid))
    }

    /// The note files waiting for `shell`.
    fn notes_of(&self, shell: &ShellId) -> Vec<PathBuf> {
        let prefix = note_prefix(shell);
        let Ok(entries) = fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with(&prefix) && name.ends_with(NOTE_SUFFIX))
            })
            .collect()
    }

    /// Writes `bytes` to `path` atomically as an owner-only file, creating the
    /// shells folder owner-only.
    ///
    /// # Errors
    /// Returns [`ShellRegistryError::Write`] naming `pid` when it fails.
    fn write(&self, pid: Pid, path: &Path, bytes: &[u8]) -> Result<(), ShellRegistryError> {
        let written = (|| -> io::Result<()> {
            create_private_dir_all(&self.dir)?;
            let mut file = AtomicWriteFile::options()
                .mode(PRIVATE_FILE_MODE)
                .preserve_mode(false)
                .open(path)?;
            file.write_all(bytes)?;
            file.commit()
        })();
        written.map_err(|error| ShellRegistryError::Write {
            pid,
            reason: error.to_string(),
        })
    }
}

impl ShellRegistry for ShellFiles {
    fn identify(&self, pid: Pid) -> Option<ShellId> {
        if !SignalTarget::Process(pid).is_alive() || LocalProcessIdentity::has_exited(pid) {
            return None;
        }
        let start = LocalProcessIdentity::start_token(pid)?;
        Some(ShellId::builder().pid(pid).start(start).build())
    }

    /// A shell whose start token cannot be read now is taken as gone: every
    /// shell was identified by one.
    fn is_live(&self, shell: &ShellId) -> bool {
        self.identify(shell.pid()) == Some(*shell)
    }

    fn records(&self) -> Vec<ShellRecord> {
        let Ok(entries) = fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.ends_with(RECORD_SUFFIX))
            })
            .filter_map(|path| read_record(&path))
            .collect()
    }

    fn record(&self, shell: &ShellId) -> Option<ShellRecord> {
        self.stored(shell.pid())
            .filter(|record| record.shell() == shell)
    }

    /// A record another process left on the same pid is never newer: it is
    /// replaced.
    fn save_if_newer(&self, record: &ShellRecord) -> Result<bool, ShellRegistryError> {
        let pid = record.shell().pid();
        if self
            .record(record.shell())
            .is_some_and(|stored| !record.stamp().supersedes(stored.stamp()))
        {
            return Ok(false);
        }
        let bytes =
            serde_json::to_vec(&record.to_wire()).map_err(|error| ShellRegistryError::Write {
                pid,
                reason: error.to_string(),
            })?;
        self.write(pid, &self.record_path(pid), &bytes)?;
        Ok(true)
    }

    /// Writes nothing for no lines, and drops the batch whole once the shell
    /// has `MAX_NOTE_FILES` waiting.
    fn deliver(
        &self,
        shell: &ShellId,
        stamp: ReportStamp,
        lines: &[String],
    ) -> Result<(), ShellRegistryError> {
        if lines.is_empty() || self.notes_of(shell).len() >= MAX_NOTE_FILES {
            return Ok(());
        }
        let text: String = lines.iter().map(|line| format!("{line}\n")).collect();
        self.write(shell.pid(), &self.note_path(shell, stamp), text.as_bytes())
    }

    /// A record a later shell on the same pid wrote is that shell's, and it
    /// stays, as do that shell's notes: only `shell`'s own go.
    fn forget(&self, shell: &ShellId) {
        if self
            .stored(shell.pid())
            .is_none_or(|stored| stored.shell() == shell)
        {
            let _ = fs::remove_file(self.record_path(shell.pid()));
        }
        for note in self.notes_of(shell) {
            let _ = fs::remove_file(note);
        }
    }

    fn notes_dir(&self) -> PathBuf {
        self.dir.clone()
    }
}

/// What every note file of `shell` is named from: its pid, which the hook
/// globs on, then its start token.
fn note_prefix(shell: &ShellId) -> String {
    format!(
        "{}{NOTE_SEPARATOR}{}{NOTE_SEPARATOR}",
        shell.pid(),
        shell.start()
    )
}

/// The record in the file at `path`, none when it cannot be read or parsed.
fn read_record(path: &Path) -> Option<ShellRecord> {
    let bytes = fs::read(path).ok()?;
    let wire: ShellRecordWire = serde_json::from_slice(&bytes).ok()?;
    ShellRecord::from_wire(wire).ok()
}

#[cfg(test)]
mod tests {
    use std::{
        os::unix::fs::PermissionsExt,
        process::{self, Command, Stdio},
    };

    use tempfile::TempDir;

    use super::*;
    use crate::domain::{
        occupancy::testing::stamp,
        value::{ProjectRoot, StartToken},
    };

    /// The permission bits of a mode.
    const PERMISSION_BITS: u32 = 0o777;
    /// A mode only the owner can enter.
    const PRIVATE_DIR_MODE: u32 = 0o700;
    /// A program that runs until it is killed.
    const SLEEPER: &str = "sleep";
    /// How long the sleeper would run, in seconds.
    const SLEEP_SECONDS: &str = "30";
    /// A start token no live process has.
    const WRONG_START: u64 = 1;
    /// The hook load the records are reported from.
    const LOAD: u64 = 7;
    /// The folder a record outside every project is reported from.
    const OUTSIDE: &str = "/";

    /// Shell files under a fresh scratch state directory.
    fn files() -> (TempDir, ShellFiles) {
        let scratch = TempDir::new().unwrap();
        let files = ShellFiles::under(scratch.path());
        (scratch, files)
    }

    /// This test process as a shell, identified for real.
    fn this_shell(files: &ShellFiles) -> ShellId {
        files
            .identify(Pid::try_new(process::id()).unwrap())
            .unwrap()
    }

    /// The record of `shell` at prompt `seq` of hook load `load` in
    /// `project`.
    fn loaded(shell: ShellId, load: u64, seq: u64, project: Option<&str>) -> ShellRecord {
        ShellRecord::builder()
            .shell(shell)
            .stamp(stamp(load, seq))
            .folder(PathBuf::from(project.unwrap_or(OUTSIDE)))
            .project(project.map(|root| ProjectRoot::try_new(PathBuf::from(root)).unwrap()))
            .build()
    }

    /// The record of `shell` at prompt `seq` of [`LOAD`] in `project`.
    fn record(shell: ShellId, seq: u64, project: Option<&str>) -> ShellRecord {
        loaded(shell, LOAD, seq, project)
    }

    /// The permission bits of `path`.
    fn mode_of(path: &Path) -> u32 {
        fs::metadata(path).unwrap().permissions().mode() & PERMISSION_BITS
    }

    /// The names of every file in `dir`, sorted.
    fn names_in(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn the_shells_dir_is_0700_and_every_file_0600() {
        let (_scratch, files) = files();
        let shell = this_shell(&files);

        files.save_if_newer(&record(shell, 1, None)).unwrap();
        files
            .deliver(&shell, stamp(LOAD, 1), &["hello".to_string()])
            .unwrap();

        assert_eq!(mode_of(&files.notes_dir()), PRIVATE_DIR_MODE);
        let names = names_in(&files.notes_dir());
        assert_eq!(names.len(), 2, "{names:?}");
        for name in names {
            assert_eq!(
                mode_of(&files.notes_dir().join(&name)),
                PRIVATE_FILE_MODE,
                "{name}"
            );
        }
    }

    #[test]
    fn a_record_is_saved_only_when_newer() {
        let (_scratch, files) = files();
        let shell = this_shell(&files);

        assert!(
            files
                .save_if_newer(&record(shell, 5, Some("/new")))
                .unwrap()
        );
        assert!(
            !files
                .save_if_newer(&record(shell, 4, Some("/old")))
                .unwrap()
        );
        assert!(
            !files
                .save_if_newer(&record(shell, 5, Some("/old")))
                .unwrap()
        );

        assert_eq!(files.record(&shell), Some(record(shell, 5, Some("/new"))));
        assert!(files.save_if_newer(&record(shell, 6, None)).unwrap());
        assert_eq!(files.record(&shell), Some(record(shell, 6, None)));
    }

    /// A shell that `exec`s itself loads the hook again and counts from one:
    /// its first report under the new load replaces the record.
    #[test]
    fn a_record_of_a_new_hook_load_is_saved_whatever_its_seq() {
        let (_scratch, files) = files();
        let shell = this_shell(&files);
        files
            .save_if_newer(&loaded(shell, LOAD, 50, Some("/old")))
            .unwrap();

        assert!(
            files
                .save_if_newer(&loaded(shell, LOAD + 1, 1, Some("/new")))
                .unwrap()
        );

        assert_eq!(
            files.record(&shell),
            Some(loaded(shell, LOAD + 1, 1, Some("/new")))
        );
    }

    #[test]
    fn records_skip_corrupt_and_unknown_key_files() {
        let (_scratch, files) = files();
        let shell = this_shell(&files);
        files.save_if_newer(&record(shell, 1, Some("/a"))).unwrap();
        fs::write(files.notes_dir().join("7.json"), b"{not json").unwrap();
        fs::write(
            files.notes_dir().join("8.json"),
            br#"{"pid":8,"start":3,"load":1,"seq":1,"folder":"/","project":null,"held":[],"extra":true}"#,
        )
        .unwrap();

        assert_eq!(files.records(), vec![record(shell, 1, Some("/a"))]);
    }

    #[test]
    fn a_record_left_by_a_reused_pid_reads_as_no_record() {
        let (_scratch, files) = files();
        let shell = this_shell(&files);
        let earlier = ShellId::builder()
            .pid(shell.pid())
            .start(StartToken::try_new(WRONG_START).unwrap())
            .build();
        files
            .save_if_newer(&record(earlier, 9, Some("/a")))
            .unwrap();

        assert_eq!(files.record(&shell), None);
        assert!(!files.is_live(&earlier));
        assert!(
            files.save_if_newer(&record(shell, 1, None)).unwrap(),
            "a record of another process on the pid is not newer"
        );
        assert_eq!(files.record(&shell), Some(record(shell, 1, None)));
    }

    #[test]
    fn identify_names_a_live_child_and_is_live_turns_false_once_it_exits() {
        let (_scratch, files) = files();
        let mut child = Command::new(SLEEPER)
            .arg(SLEEP_SECONDS)
            .stdin(Stdio::null())
            .spawn()
            .unwrap();
        let pid = Pid::try_new(child.id()).unwrap();

        let shell = files.identify(pid).unwrap();
        assert_eq!(shell.pid(), pid);
        assert!(files.is_live(&shell));

        child.kill().unwrap();
        child.wait().unwrap();
        assert!(!files.is_live(&shell));
        assert_eq!(files.identify(pid), None);
    }

    #[test]
    fn deliver_writes_one_note_file_per_call_and_stops_at_the_cap() {
        let (_scratch, files) = files();
        let shell = this_shell(&files);
        let lines = vec!["one".to_string(), "two".to_string()];

        files.deliver(&shell, stamp(LOAD, 3), &lines).unwrap();
        files.deliver(&shell, stamp(LOAD, 4), &[]).unwrap();

        let first = files.note_path(&shell, stamp(LOAD, 3));
        assert_eq!(fs::read_to_string(&first).unwrap(), "one\ntwo\n");
        assert_eq!(
            first.file_name().unwrap().to_string_lossy(),
            format!(
                "{}-{}-00000000000000000007-00000000000000000003.note",
                shell.pid(),
                shell.start()
            )
        );
        assert_eq!(files.notes_of(&shell).len(), 1, "no lines, no file");

        for seq in 10..10 + MAX_NOTE_FILES as u64 {
            files.deliver(&shell, stamp(LOAD, seq), &lines).unwrap();
        }
        assert_eq!(files.notes_of(&shell).len(), MAX_NOTE_FILES);
        assert!(
            !files
                .note_path(&shell, stamp(LOAD, 10 + MAX_NOTE_FILES as u64 - 1))
                .exists(),
            "the batch past the cap is dropped"
        );
    }

    #[test]
    fn forget_removes_the_record_and_its_notes_only() {
        let (_scratch, files) = files();
        let shell = this_shell(&files);
        let other = ShellId::builder()
            .pid(Pid::try_new(shell.pid().into_inner() * 10 + 1).unwrap())
            .start(StartToken::try_new(WRONG_START).unwrap())
            .build();
        for each in [shell, other] {
            files.save_if_newer(&record(each, 1, None)).unwrap();
            files
                .deliver(&each, stamp(LOAD, 1), &["note".to_string()])
                .unwrap();
        }

        files.forget(&shell);

        assert_eq!(files.record(&shell), None);
        assert!(files.notes_of(&shell).is_empty());
        assert_eq!(files.record(&other), Some(record(other, 1, None)));
        assert_eq!(files.notes_of(&other).len(), 1);
    }

    /// Concurrency M3: forgetting a shell gone since leaves the record and
    /// the notes a later shell on its pid wrote, even notes written before
    /// that shell's first record, and still takes the earlier one's notes.
    #[test]
    fn forget_leaves_a_later_shell_on_the_same_pid_alone() {
        let (_scratch, files) = files();
        let shell = this_shell(&files);
        let earlier = ShellId::builder()
            .pid(shell.pid())
            .start(StartToken::try_new(WRONG_START).unwrap())
            .build();
        files
            .deliver(&earlier, stamp(LOAD, 9), &["stale".to_string()])
            .unwrap();
        files
            .deliver(&shell, stamp(LOAD, 1), &["fresh".to_string()])
            .unwrap();

        files.forget(&earlier);

        assert!(files.notes_of(&earlier).is_empty());
        assert_eq!(files.notes_of(&shell).len(), 1, "the later shell's note");

        files.save_if_newer(&record(shell, 1, None)).unwrap();
        files.forget(&earlier);
        assert_eq!(files.record(&shell), Some(record(shell, 1, None)));
        assert_eq!(files.notes_of(&shell).len(), 1);
    }
}
