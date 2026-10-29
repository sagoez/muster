//! Spawn requests the holder, runner and service tests launch.

use std::{path::PathBuf, time::Duration};

use bytesize::ByteSize as RawByteSize;

use crate::domain::{
    color_scheme::ColorScheme,
    process::{StopPolicy, StopSignal},
    pty::{PtySize, SpawnRequest},
    value::{ByteSize, CommandLine, ProcessName, StopGrace},
};

/// PTY rows of every test child.
pub(crate) const TEST_ROWS: u16 = 24;
/// PTY cols of every test child.
pub(crate) const TEST_COLS: u16 = 80;
/// Scrollback every test terminal keeps.
pub(crate) const TEST_SCROLLBACK: usize = 100;
/// Log cap far above anything a test writes, in MiB.
const TEST_LOG_CAP_MIB: u64 = 64;
/// Grace every test child's stop policy gives it.
pub(crate) const TEST_STOP_GRACE: Duration = Duration::from_millis(500);

/// The terminal size every test child starts with.
pub(crate) fn test_size() -> PtySize {
    PtySize::try_new(TEST_ROWS, TEST_COLS).unwrap()
}

/// The stop policy every test child is launched with.
pub(crate) fn test_stop() -> StopPolicy {
    StopPolicy::builder()
        .signal(StopSignal::Terminate)
        .grace_period(StopGrace::try_new(TEST_STOP_GRACE).unwrap())
        .build()
}

/// A request to run `command` as `process` of the project at `project`, in
/// `working_dir` when given, with every launch setting explicit.
pub(crate) fn test_request(
    process: &str,
    command: &str,
    project: Option<PathBuf>,
    working_dir: Option<PathBuf>,
) -> SpawnRequest {
    SpawnRequest::builder()
        .process(ProcessName::try_new(process).unwrap())
        .command(CommandLine::try_new(command).unwrap())
        .project(project)
        .working_dir(working_dir)
        .size(test_size())
        .scrollback(TEST_SCROLLBACK)
        .log_max_size(ByteSize::try_new(RawByteSize::mib(TEST_LOG_CAP_MIB)).unwrap())
        .color_scheme(ColorScheme::Dark)
        .stop(test_stop())
        .build()
}
