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
        StackEvent, StatusAnswer,
    },
};

use super::{
    model::Floor,
    reading::{ProjectReading, StackedProcess, StatusReading},
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
/// A start says its run armed no readiness check.
pub(crate) fn stack_event(project: &str, name: &str, kind: EventKind) -> StackEvent {
    logged_event(project, name, kind, next_seq())
}

/// The event [`stack_event`] describes, numbered `seq` instead.
pub(crate) fn logged_event(
    project: &str,
    name: &str,
    kind: EventKind,
    seq: EventSeq,
) -> StackEvent {
    let waits_ready = (kind == EventKind::Started).then_some(false);
    event_of(project, name, kind, seq, waits_ready)
}

/// The start the service logged for process `name` of project `project`,
/// as [`stack_event`] makes it, saying its run armed a readiness check when
/// `waits_ready`.
pub(crate) fn started_event(project: &str, name: &str, waits_ready: bool) -> StackEvent {
    event_of(
        project,
        name,
        EventKind::Started,
        next_seq(),
        Some(waits_ready),
    )
}

/// The number the next fixture event of the fixture log takes.
fn next_seq() -> EventSeq {
    let seq = NEXT_EVENT_SEQ.fetch_add(1, Ordering::SeqCst);
    EventSeq::builder().epoch(EVENT_EPOCH).seq(seq).build()
}

/// The event of `kind` numbered `seq` for process `name` of `project`,
/// saying `waits_ready` of readiness.
fn event_of(
    project: &str,
    name: &str,
    kind: EventKind,
    seq: EventSeq,
    waits_ready: Option<bool>,
) -> StackEvent {
    StackEvent::builder()
        .seq(seq)
        .at(UNIX_EPOCH)
        .project(Some(root(project)))
        .process(Some(ProcessName::try_new(name).unwrap()))
        .kind(kind)
        .detail(String::new())
        .cursor(None)
        .waits_ready(waits_ready)
        .build()
}

/// The newest fixture event made so far, as a service stamps its status
/// answer: every event made before is at or before it, every later one
/// after.
pub(crate) fn newest_event() -> EventSeq {
    let seq = NEXT_EVENT_SEQ.load(Ordering::SeqCst) - 1;
    EventSeq::builder().epoch(EVENT_EPOCH).seq(seq).build()
}

/// The fixture service's answer holding `projects`, stamped with the
/// newest event made so far.
pub(crate) fn answered(projects: Vec<ProjectStatus>) -> StatusReading {
    answered_at(projects, newest_event())
}

/// The fixture service's answer holding `projects`, stamped `cursor`.
pub(crate) fn answered_at(projects: Vec<ProjectStatus>, cursor: EventSeq) -> StatusReading {
    StatusReading::Answered(
        StatusAnswer::builder()
            .projects(projects)
            .cursor(cursor)
            .build(),
    )
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
    HolderDirs::placed(Path::new(STATE_DIR))
}

/// An empty floor started inside `shop`, with paths under the fixture home.
pub(crate) fn floor() -> Floor {
    Floor::new(PathBuf::from(CWD), Some(PathBuf::from(HOME)), holder_dirs())
}
