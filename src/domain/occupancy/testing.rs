//! What tests of shells and their projects share: a shell by pid and start,
//! a project folder by name, and a report's stamp.

use std::path::PathBuf;

use super::{LoadId, LoadStart, ReportSeq, ReportStamp, ShellId};
use crate::domain::value::{Pid, ProjectRoot, StartToken};

/// The folder test projects live under.
const TEST_HOME: &str = "/home/me";
/// The pid a test hook load spawned, unless a test breaks a tie with it.
const TEST_LOAD_PID: u32 = 2;

/// The shell running as `pid`, started at `start`.
pub(crate) fn test_shell(pid: u32, start: u64) -> ShellId {
    ShellId::builder()
        .pid(Pid::try_new(pid).unwrap())
        .start(StartToken::try_new(start).unwrap())
        .build()
}

/// The project folder `/home/me/<name>`.
pub(crate) fn test_root(name: &str) -> ProjectRoot {
    ProjectRoot::try_new(PathBuf::from(TEST_HOME).join(name)).unwrap()
}

/// The stamp of prompt `seq` of the hook load begun at `load`.
pub(crate) fn stamp(load: u64, seq: u64) -> ReportStamp {
    loaded_stamp(load, TEST_LOAD_PID, seq)
}

/// The stamp of prompt `seq` of the hook load begun at `load` that spawned
/// `pid`.
pub(crate) fn loaded_stamp(load: u64, pid: u32, seq: u64) -> ReportStamp {
    ReportStamp::builder()
        .load(
            LoadId::builder()
                .started(LoadStart::new(load))
                .pid(Pid::try_new(pid).unwrap())
                .build(),
        )
        .seq(ReportSeq::new(seq))
        .build()
}
