use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

use getset::Getters;
use muster_domain::value::ProjectRoot;
use typed_builder::TypedBuilder;

use crate::holder::launch::{
    id::HolderId,
    record::{HolderRecord, read_records},
};

/// The holder records in a folder, read once and sorted by whether their
/// holder still serves and, for one that does not, whether it left its
/// child's exit, so a service start and doctor judge every holder by one
/// reading and one rule, and no later pass asks the system again.
#[derive(Debug, Clone, Default, Getters, TypedBuilder)]
pub struct RecordScan {
    /// Records whose holder still serves, as [`HolderRecord::is_serving`]
    /// tells, one binding its socket again included.
    #[getset(get = "pub")]
    serving: Vec<HolderRecord>,
    /// Records whose holder no longer serves and left no exit.
    #[getset(get = "pub")]
    unserved: Vec<HolderRecord>,
    /// Records whose holder no longer serves and left its child's exit
    /// beside them, for the next service to report.
    #[getset(get = "pub")]
    stranded: Vec<HolderRecord>,
    /// Record files that cannot be read or parsed.
    #[getset(get = "pub")]
    unreadable: Vec<PathBuf>,
}

impl RecordScan {
    /// Reads every record in `dir` of `project`, or of every project when
    /// none, once and probes each one's holder. An exit left beside a
    /// serving holder's record is never taken for its end. A missing folder
    /// holds no records.
    pub fn read(dir: &Path, project: Option<&ProjectRoot>) -> Self {
        let mut scan = Self::default();
        for (path, record) in read_records(dir) {
            match record {
                Some(record) if project.is_some_and(|project| record.project() != project) => {},
                Some(record) if record.is_serving() => scan.serving.push(record),
                Some(record) if record.left_exit(dir).is_some() => scan.stranded.push(record),
                Some(record) => scan.unserved.push(record),
                None => scan.unreadable.push(path),
            }
        }
        scan
    }

    /// This scan keeping only the unserved records `keep` accepts.
    pub fn retaining_unserved(mut self, keep: impl FnMut(&HolderRecord) -> bool) -> Self {
        self.unserved.retain(keep);
        self
    }

    /// Every readable record, in the order to adopt them: serving, then
    /// unserved, then stranded, so a live copy of a process is always
    /// adopted before an exit left of it.
    pub fn live(&self) -> Vec<HolderRecord> {
        self.serving
            .iter()
            .chain(&self.unserved)
            .chain(&self.stranded)
            .cloned()
            .collect()
    }

    /// Whether the holder of `record` left its child's exit.
    pub fn is_stranded(&self, record: &HolderRecord) -> bool {
        self.stranded.iter().any(|left| left.id() == record.id())
    }

    /// The ids of the holders that still serve.
    pub fn serving_ids(&self) -> HashSet<HolderId> {
        self.serving
            .iter()
            .map(|record| record.id().clone())
            .collect()
    }

    /// The ids the unreadable record files are named for: holders that may
    /// well be running, though nothing can tell.
    pub fn unreadable_ids(&self) -> HashSet<HolderId> {
        self.unreadable
            .iter()
            .filter_map(|path| HolderId::of_file(path))
            .collect()
    }

    /// The ids whose sockets a sweep keeps: every serving holder's, and
    /// every unreadable record's, since its holder may be running.
    pub fn kept_ids(&self) -> HashSet<HolderId> {
        let mut kept = self.serving_ids();
        kept.extend(self.unreadable_ids());
        kept
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        os::unix::net::{UnixDatagram, UnixListener},
        process::{self, Command},
        time::{Duration, SystemTime},
    };

    use muster_domain::{
        process::{StopPolicy, StopSignal},
        pty::ExitOutcome,
        value::{CommandLine, OutputCursor, Pid, ProcessName, StopGrace},
    };
    use muster_os::{process_identity::LocalProcessIdentity, test_scratch::short_scratch};

    use super::*;
    use crate::holder::launch::{left_exit::LeftExit, record::RECORD_EXTENSION};

    /// Prefix of each test's scratch directory, short so sockets fit.
    const SCRATCH_PREFIX: &str = "rs-";
    /// A command that exits at once, run to get a pid no process holds.
    const EXITS_AT_ONCE: &str = "true";
    /// The process every test record names.
    const PROCESS: &str = "api";
    /// Text no record parses from.
    const CORRUPT: &[u8] = b"{not json";
    /// Grace of every test record's stop.
    const STOP_GRACE: Duration = Duration::from_secs(1);
    /// A project folder no test record names.
    const OTHER_PROJECT: &str = "elsewhere";

    /// The test process's own pid.
    fn own_pid() -> Pid {
        Pid::try_new(process::id()).unwrap()
    }

    /// A pid no process holds any more.
    fn dead_pid() -> Pid {
        let mut ended = Command::new(EXITS_AT_ONCE).spawn().unwrap();
        let pid = Pid::try_new(ended.id()).unwrap();
        ended.wait().unwrap();
        pid
    }

    /// Saves in `dir` the record of holder `id` running as `holder_pid` at
    /// `socket`.
    fn save(dir: &Path, id: HolderId, holder_pid: Pid, socket: PathBuf) -> HolderRecord {
        let root = dir.parent().unwrap().to_path_buf();
        let record = HolderRecord::builder()
            .id(id)
            .project(ProjectRoot::try_new(root.clone()).unwrap())
            .process(ProcessName::try_new(PROCESS).unwrap())
            .command(CommandLine::try_new(EXITS_AT_ONCE).unwrap())
            .stop(
                StopPolicy::builder()
                    .signal(StopSignal::Terminate)
                    .grace_period(StopGrace::try_new(STOP_GRACE).unwrap())
                    .build(),
            )
            .socket(socket)
            .holder_pid(holder_pid)
            .child_pid(holder_pid)
            .child_start_token(None)
            .log(root.join(PROCESS))
            .child_started_at(SystemTime::now())
            .build();
        record.save(dir).unwrap();
        record
    }

    /// A holder recorded with no start token serves only while its pid is
    /// alive and its socket accepts: a live pid whose socket refuses, as a
    /// reused pid's does, is unserved like a dead holder, and an unreadable
    /// record is neither, though a sweep keeps its socket.
    #[test]
    fn records_are_sorted_by_the_adoption_rule() {
        let scratch = short_scratch(SCRATCH_PREFIX);
        let dir = scratch.path().join("records");
        let serving_socket = scratch.path().join("serving");
        let _accepting = UnixListener::bind(&serving_socket).unwrap();
        let serving = save(
            &dir,
            HolderId::generate().unwrap(),
            own_pid(),
            serving_socket,
        );
        let refusing_socket = scratch.path().join("refusing");
        let _refusing = UnixDatagram::bind(&refusing_socket).unwrap();
        let reused = save(
            &dir,
            HolderId::generate().unwrap(),
            own_pid(),
            refusing_socket,
        );
        let dead = save(
            &dir,
            HolderId::generate().unwrap(),
            dead_pid(),
            scratch.path().join("gone"),
        );
        let unreadable = HolderId::generate().unwrap();
        let unreadable_file = dir
            .join(unreadable.as_ref())
            .with_extension(RECORD_EXTENSION);
        fs::write(&unreadable_file, CORRUPT).unwrap();

        let scan = RecordScan::read(&dir, None);

        assert_eq!(scan.serving(), &vec![serving.clone()]);
        let unserved: HashSet<HolderId> = scan
            .unserved()
            .iter()
            .map(|record| record.id().clone())
            .collect();
        assert_eq!(
            unserved,
            HashSet::from([reused.id().clone(), dead.id().clone()])
        );
        assert_eq!(scan.unreadable(), &vec![unreadable_file]);
        assert_eq!(scan.serving_ids(), HashSet::from([serving.id().clone()]));
        assert_eq!(
            scan.kept_ids(),
            HashSet::from([serving.id().clone(), unreadable])
        );
        let kept = scan.retaining_unserved(|record| record.id() == dead.id());
        assert_eq!(kept.live(), vec![serving, dead]);
    }

    /// A holder its start token says is still itself serves with its socket
    /// gone, as while it binds it again; a dead holder that left its child's
    /// exit is stranded, adopted after the rest; and a scan of one project
    /// reads no other project's records.
    #[test]
    fn a_holder_alive_by_its_token_serves_without_its_socket() {
        let Some(token) = LocalProcessIdentity::start_token(own_pid()) else {
            return;
        };
        let scratch = short_scratch(SCRATCH_PREFIX);
        let dir = scratch.path().join("records");
        let rebinding = HolderRecord::builder()
            .holder_start_token(Some(token))
            .boot_id(LocalProcessIdentity::boot_id())
            .id(HolderId::generate().unwrap())
            .project(ProjectRoot::try_new(scratch.path().to_path_buf()).unwrap())
            .process(ProcessName::try_new(PROCESS).unwrap())
            .command(CommandLine::try_new(EXITS_AT_ONCE).unwrap())
            .stop(
                StopPolicy::builder()
                    .signal(StopSignal::Terminate)
                    .grace_period(StopGrace::try_new(STOP_GRACE).unwrap())
                    .build(),
            )
            .socket(scratch.path().join("gone"))
            .holder_pid(own_pid())
            .child_pid(own_pid())
            .child_start_token(None)
            .log(scratch.path().join(PROCESS))
            .child_started_at(SystemTime::now())
            .build();
        rebinding.save(&dir).unwrap();
        let stranded = save(
            &dir,
            HolderId::generate().unwrap(),
            dead_pid(),
            scratch.path().join("left"),
        );
        let left = LeftExit::builder()
            .outcome(ExitOutcome::FAILED_UNKNOWN)
            .run_start(OutputCursor::new(0))
            .build();
        stranded.save_stranded(&dir, &left).unwrap();

        let scan = RecordScan::read(&dir, None);

        assert_eq!(scan.serving(), &vec![rebinding.clone()]);
        assert!(scan.unserved().is_empty());
        assert!(scan.is_stranded(&stranded) && !scan.is_stranded(&rebinding));
        assert_eq!(scan.live(), vec![rebinding, stranded]);
        let elsewhere = ProjectRoot::try_new(scratch.path().join(OTHER_PROJECT)).unwrap();
        assert!(RecordScan::read(&dir, Some(&elsewhere)).live().is_empty());
    }
}
