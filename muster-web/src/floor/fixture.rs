use std::{
    net::{IpAddr, Ipv4Addr},
    path::{Path, PathBuf},
    time::Duration,
};

use muster_domain::{
    port::ListeningSocket,
    process::{ProcessIcon, ProcessState, Readiness},
    project::Project,
    value::{IdleStop, Pid, ProcessName, ProjectName, ProjectRoot},
};
use muster_protocol::{
    holder::HolderDirs,
    service::{OccupancyStatus, ProcessStatus, ProjectStatus},
};

use super::{
    model::Floor,
    reading::{ProjectReading, StackedProcess},
};

/// The fixture user's home folder, which every fixture project lives in.
pub(super) const HOME: &str = "/home/me";
/// The folder the fixture view is started in, inside `shop`.
const CWD: &str = "/home/me/shop/api";
/// The state directory the fixture's logs live under.
const STATE_DIR: &str = "/state";
/// The pid every fixture socket is held by.
const SOCKET_PID: u32 = 4242;
/// Seconds the fixture's processes get to stop.
const GRACE_SECONDS: u64 = 5;

/// The folder of the fixture project `name`.
pub(super) fn root(name: &str) -> ProjectRoot {
    ProjectRoot::try_new(Path::new(HOME).join(name)).unwrap()
}

/// The fixture project `name`, in its folder under the home folder.
pub(super) fn project(name: &str) -> Project {
    Project::builder()
        .root(root(name))
        .name(ProjectName::try_new(name).unwrap())
        .build()
}

/// The reading of project `name` holding `processes`, each wearing `icon`.
pub(super) fn project_reading_with(
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
        .build()
}

/// The reading of project `name` holding `processes`, every icon a terminal.
pub(super) fn project_reading(name: &str, processes: &[&str]) -> ProjectReading {
    project_reading_with(name, processes, ProcessIcon::Terminal)
}

/// Process `name` in `state` with readiness `ready`, listening on `ports`
/// after `restarts` restarts.
pub(super) fn process_status_with(
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
pub(super) fn process_status(name: &str, state: ProcessState, ready: Readiness) -> ProcessStatus {
    process_status_with(name, state, ready, &[], 0)
}

/// Project `name`'s status, holding `processes`.
pub(super) fn project_status(name: &str, processes: Vec<ProcessStatus>) -> ProjectStatus {
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
pub(super) fn holder_dirs() -> HolderDirs {
    HolderDirs::under(Path::new(STATE_DIR))
}

/// An empty floor started inside `shop`, with paths under the fixture home.
pub(super) fn floor() -> Floor {
    Floor::new(PathBuf::from(CWD), Some(PathBuf::from(HOME)), holder_dirs())
}
