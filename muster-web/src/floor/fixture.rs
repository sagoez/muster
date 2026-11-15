use std::{
    net::{IpAddr, Ipv4Addr},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, UNIX_EPOCH},
};

use muster_domain::{
    port::ListeningSocket,
    process::{ProcessIcon, ProcessState, Readiness},
    project::Project,
    value::{IdleStop, Pid, ProcessName, ProjectName, ProjectRoot},
};
use muster_protocol::{
    holder::HolderDirs,
    service::{
        EventKind, EventSeq, OccupancyStatus, ProcessStatus, ProcessUsage, ProjectStatus,
        StackEvent,
    },
};

use super::{
    model::Floor,
    reading::{ProjectReading, StackedProcess},
};

/// The fixture user's home folder, which every fixture project lives in.
pub(crate) const HOME: &str = "/home/me";
/// The folder the fixture view is started in, inside `shop`.
const CWD: &str = "/home/me/shop/api";
/// The state directory the fixture's logs live under.
const STATE_DIR: &str = "/state";
/// The pid every fixture socket is held by.
const SOCKET_PID: u32 = 4242;
/// Seconds the fixture's processes get to stop.
const GRACE_SECONDS: u64 = 5;
/// The epoch of the fixture service's event log.
const EVENT_EPOCH: u32 = 1;

/// The number the next fixture event takes in that log, rising across every
/// test so events made in turn are numbered in turn.
static NEXT_EVENT_SEQ: AtomicU64 = AtomicU64::new(1);

/// The folder of the fixture project `name`.
pub(crate) fn root(name: &str) -> ProjectRoot {
    ProjectRoot::try_new(Path::new(HOME).join(name)).unwrap()
}

/// The fixture project `name`, in its folder under the home folder.
pub(crate) fn project(name: &str) -> Project {
    Project::builder()
        .root(root(name))
        .name(ProjectName::try_new(name).unwrap())
        .build()
}

/// The reading of project `name` holding `processes`, each wearing `icon`.
pub(crate) fn project_reading_with(
    name: &str,
    processes: &[&str],
    icon: ProcessIcon,
) -> ProjectReading {
    let processes = processes
        .iter()
        .map(|process| {
            StackedProcess::builder()
                .name(ProcessName::try_new(*process).unwrap())
                .icon(icon)
                .build()
        })
        .collect();
    ProjectReading::builder()
        .project(project(name))
        .icon(icon)
        .processes(processes)
        .unreadable(false)
        .build()
}

/// The reading of project `name` holding `processes`, every icon a terminal.
pub(crate) fn project_reading(name: &str, processes: &[&str]) -> ProjectReading {
    project_reading_with(name, processes, ProcessIcon::Terminal)
}

/// Process `name` in `state` with readiness `ready`, listening on `ports`
/// after `restarts` restarts.
pub(crate) fn process_status_with(
    name: &str,
    state: ProcessState,
    ready: Readiness,
    ports: &[u16],
    restarts: u32,
) -> ProcessStatus {
    let sockets = ports
        .iter()
        .map(|port| {
            ListeningSocket::builder()
                .port(*port)
                .address(IpAddr::V4(Ipv4Addr::LOCALHOST))
                .pid(Pid::try_new(SOCKET_PID).unwrap())
                .build()
        })
        .collect();
    ProcessStatus::builder()
        .name(ProcessName::try_new(name).unwrap())
        .state(state)
        .ready(ready)
        .pid(None)
        .uptime(None)
        .restarts(restarts)
        .usage(None)
        .attached(0)
        .ports(sockets)
        .last_exit(None)
        .stop_grace(Duration::from_secs(GRACE_SECONDS))
        .build()
}

/// Process `name` in `state` with readiness `ready`, on no port and never
/// restarted.
pub(crate) fn process_status(name: &str, state: ProcessState, ready: Readiness) -> ProcessStatus {
    process_status_with(name, state, ready, &[], 0)
}

/// Process `name` running with no check for `uptime`, its tree using `cpu`
/// percent and `memory` bytes.
pub(crate) fn running_with_usage(
    name: &str,
    uptime: Duration,
    cpu: f32,
    memory: u64,
) -> ProcessStatus {
    let pid = Pid::try_new(SOCKET_PID).unwrap();
    ProcessStatus::builder()
        .name(ProcessName::try_new(name).unwrap())
        .state(ProcessState::Running)
        .ready(Readiness::None)
        .pid(Some(pid))
        .uptime(Some(uptime))
        .restarts(0)
        .usage(Some(
            ProcessUsage::builder()
                .pid(pid)
                .cpu_percent(cpu)
                .memory_bytes(memory)
                .build(),
        ))
        .attached(0)
        .last_exit(None)
        .stop_grace(Duration::from_secs(GRACE_SECONDS))
        .build()
}

/// The event of `kind` the service logged for process `name` of project
/// `project`, long ago, numbered after every fixture event made before it.
pub(crate) fn stack_event(project: &str, name: &str, kind: EventKind) -> StackEvent {
    let seq = NEXT_EVENT_SEQ.fetch_add(1, Ordering::SeqCst);
    StackEvent::builder()
        .seq(EventSeq::builder().epoch(EVENT_EPOCH).seq(seq).build())
        .at(UNIX_EPOCH)
        .project(Some(root(project)))
        .process(Some(ProcessName::try_new(name).unwrap()))
        .kind(kind)
        .detail(String::new())
        .cursor(None)
        .build()
}

/// Project `name`'s status, holding `processes`.
pub(crate) fn project_status(name: &str, processes: Vec<ProcessStatus>) -> ProjectStatus {
    ProjectStatus::builder()
        .project(project(name))
        .processes(processes)
        .occupancy(
            OccupancyStatus::builder()
                .shells(0)
                .idle_stop(IdleStop::Never)
                .stops_in(None)
                .build(),
        )
        .build()
}

/// Where the fixture's holders keep their files.
pub(crate) fn holder_dirs() -> HolderDirs {
    HolderDirs::under(Path::new(STATE_DIR))
}

/// An empty floor started inside `shop`, with paths under the fixture home.
pub(crate) fn floor() -> Floor {
    Floor::new(PathBuf::from(CWD), Some(PathBuf::from(HOME)), holder_dirs())
}
