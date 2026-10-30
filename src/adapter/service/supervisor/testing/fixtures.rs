//! The values every supervisor test starts from: the harness project, its
//! processes and settings, and the launch results a worker posts.

use std::{path::PathBuf, time::Duration};

use bytesize::ByteSize as RawByteSize;

use super::harness::Harness;
use crate::{
    adapter::service::{ServiceEvent, supervisor::SpawnGeneration},
    domain::{
        color_scheme::ColorScheme,
        config::{Environment, McpStackEdits, ProcessSpec, Stack},
        port::ProcessHandle,
        process::{RestartPolicy, StopPolicy, StopSignal},
        project::Project,
        pty::{PtyError, PtySize},
        settings::Settings,
        value::{
            ByteSize, CommandLine, DetachKey, LogMaxSize, PaneId, Pid, ProcessName, ProjectName,
            ProjectRoot, ScrollbackLines, StopGrace,
        },
    },
};

/// Folder of the project every harness supervisor runs.
pub(crate) const TEST_PROJECT: &str = "/here";
/// Terminal rows and columns every harness supervisor starts processes with.
const TEST_TERMINAL_SIDE: u16 = 24;
/// Scrollback lines every harness launch asks for.
const TEST_SCROLLBACK: usize = 100;
/// Log cap of every harness launch, in MiB.
const TEST_LOG_CAP_MIB: u64 = 4;
/// Grace of every harness process's stop policy.
pub(crate) const TEST_STOP_GRACE: Duration = Duration::from_secs(7);
/// The pid a [`PidHandle`](super::pid_handle::PidHandle) reports.
const FAKE_LAUNCHER_PID: u32 = 4242;
/// The screen a [`ConnectedHandle`](super::connected_handle::ConnectedHandle)
/// connects to.
pub(crate) const FIRST_SCREEN: &[u8] = b"first ";
/// What a [`BacklogHandle`](super::backlog_handle::BacklogHandle)'s process
/// logged before it connected: a marker on its first line.
pub(crate) const BACKLOG: &[u8] = b"READY on 3000\nbooting\n";
/// The log offset a [`BacklogHandle`](super::backlog_handle::BacklogHandle)'s
/// run began at.
pub(crate) const BACKLOG_RUN_START: u64 = 4096;
/// Where a [`LoggedAheadHandle`](super::logged_ahead_handle::LoggedAheadHandle)'s
/// log ends.
pub(crate) const LOGGED_AHEAD_END: u64 = 64;
/// Where a [`ReplayAskHandle`](super::replay_ask_handle::ReplayAskHandle)'s
/// streamed output begins.
pub(crate) const REPLAY_STREAM_START: u64 = 4096;
/// Longest a test waits for a worker's launch result.
const WORKER_WAIT: Duration = Duration::from_secs(5);
/// Why a refusing adopter fails.
pub(crate) const REFUSAL: &str = "holder unreachable";
/// Longest a test waits for an unsupervised stop to settle.
pub(crate) const SETTLE_WAIT: Duration = Duration::from_secs(10);

/// The process name `raw`.
pub(crate) fn name(raw: &str) -> ProcessName {
    ProcessName::try_new(raw).unwrap()
}

/// The project every harness supervisor runs.
pub(crate) fn test_project() -> Project {
    let root = ProjectRoot::try_new(PathBuf::from(TEST_PROJECT)).unwrap();
    let name = ProjectName::candidates(&root).next().unwrap();
    Project::builder().name(name).root(root).build()
}

/// The stop policy every harness process has: an interrupt, then a kill
/// after [`TEST_STOP_GRACE`].
pub(crate) fn test_stop() -> StopPolicy {
    StopPolicy::builder()
        .signal(StopSignal::Interrupt)
        .grace_period(StopGrace::try_new(TEST_STOP_GRACE).unwrap())
        .build()
}

/// An autostarting process `raw` running `command` under `restart`.
pub(crate) fn spec(raw: &str, command: &str, restart: RestartPolicy) -> ProcessSpec {
    ProcessSpec::builder()
        .name(name(raw))
        .command(CommandLine::try_new(command).unwrap())
        .environment(Environment::empty())
        .autostart(true)
        .restart(restart)
        .stop(test_stop())
        .build()
}

/// The harness project's stack of `processes`.
pub(crate) fn stack(processes: &[ProcessSpec]) -> Stack {
    Stack::builder()
        .project(test_project())
        .processes(processes.to_vec())
        .build()
}

/// Every setting explicit, desktop notifications off.
pub(crate) fn test_settings() -> Settings {
    Settings::builder()
        .desktop_notifications(false)
        .default_size(PtySize::try_new(TEST_TERMINAL_SIDE, TEST_TERMINAL_SIDE).unwrap())
        .detach_key(DetachKey::try_new("ctrl-\\").unwrap())
        .log_max_size(
            LogMaxSize::try_new(ByteSize::try_new(RawByteSize::mib(TEST_LOG_CAP_MIB)).unwrap())
                .unwrap(),
        )
        .scrollback(ScrollbackLines::try_new(TEST_SCROLLBACK).unwrap())
        .color_scheme(ColorScheme::Dark)
        .default_autostart(false)
        .default_restart(RestartPolicy::Never)
        .default_stop_signal(StopSignal::Terminate)
        .default_stop_grace(StopGrace::try_new(TEST_STOP_GRACE).unwrap())
        .mcp_stack_edits(McpStackEdits::Allow)
        .build()
}

/// The pid a [`PidHandle`](super::pid_handle::PidHandle) reports, as a
/// [`Pid`].
pub(crate) fn fake_launcher_pid() -> Pid {
    Pid::try_new(FAKE_LAUNCHER_PID).unwrap()
}

/// The launch result a worker posted for `harness`, which the test holds
/// until it chooses to settle it, as a gated launch would.
pub(crate) fn spawned(
    harness: &Harness,
) -> (
    PaneId,
    SpawnGeneration,
    Result<Box<dyn ProcessHandle>, PtyError>,
) {
    let Ok(ServiceEvent::Spawned {
        pane,
        generation,
        result,
        ..
    }) = harness.events().recv_timeout(WORKER_WAIT)
    else {
        panic!("the worker posts the launch result");
    };
    (pane, generation, result)
}
