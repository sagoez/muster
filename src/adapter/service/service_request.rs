use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::{
    launch_environment::LaunchEnvironment,
    process_action::ProcessAction,
    service_version::ServiceVersion,
    stack_event::{EventKind, EventSeq},
    wait_condition::WaitSpec,
};
use crate::{
    adapter::bridge::WorkspaceRequest,
    domain::value::{ProcessName, ProjectRoot},
};

/// One request a client sends the service, as one JSON line. A `project` is
/// that project's folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ServiceRequest {
    /// Introduces the client's build, so a client of another version can
    /// replace the service.
    Hello {
        /// The client's muster version.
        version: ServiceVersion,
    },
    /// Starts the project's `autostart` processes that are not running.
    Up {
        /// The project to bring up.
        project: ProjectRoot,
        /// The client's environment, which what it starts runs with.
        environment: LaunchEnvironment,
        /// A wait on each process it starts, answered as one batch once every
        /// wait ended.
        wait: Option<WaitSpec>,
    },
    /// Stops every process of the project.
    Down {
        /// The project to bring down.
        project: ProjectRoot,
    },
    /// Reports every process of one project, or of every loaded project.
    Ps {
        /// The project to report, or every loaded project when absent.
        project: Option<ProjectRoot>,
    },
    /// Applies a lifecycle action to one process.
    Control {
        /// The process's project.
        project: ProjectRoot,
        /// The process.
        name: ProcessName,
        /// What to do to it.
        action: ProcessAction,
        /// The client's environment, which a process it starts or restarts
        /// runs with.
        environment: LaunchEnvironment,
        /// A wait on the run the action leaves, registered in the same loop
        /// turn as the action so none of that run's output is missed; the
        /// answer is then how the wait ended.
        wait: Option<WaitSpec>,
    },
    /// Waits for a process's output, port, URL, state or readiness.
    Wait {
        /// The process's project.
        project: ProjectRoot,
        /// The process.
        name: ProcessName,
        /// What to wait for, and for how long.
        wait: WaitSpec,
    },
    /// Reads the project's lifecycle events after a cursor, waiting for one
    /// when there are none yet.
    Events {
        /// The project whose events to read.
        project: ProjectRoot,
        /// Events after this one count; without it, the newest few.
        since: Option<EventSeq>,
        /// The kinds to read, every kind when empty.
        kinds: Vec<EventKind>,
        /// Longest to wait for a new event when there is none.
        #[serde(with = "humantime_serde")]
        wait: Duration,
    },
    /// Finds where a running process's holder and log are.
    Locate {
        /// The process's project.
        project: ProjectRoot,
        /// The process.
        name: ProcessName,
    },
    /// Finds the log every run of a process writes to, whether or not it is
    /// running.
    Log {
        /// The process's project.
        project: ProjectRoot,
        /// The process.
        name: ProcessName,
    },
    /// Reads a process's visible screen.
    Screen {
        /// The process's project.
        project: ProjectRoot,
        /// The process.
        name: ProcessName,
    },
    /// Reports the recent notices of a project and of the service.
    Notices {
        /// The project whose notices to report.
        project: ProjectRoot,
    },
    /// A workspace request from an MCP client.
    Workspace {
        /// The project the client serves.
        project: ProjectRoot,
        /// The request.
        request: WorkspaceRequest,
    },
    /// The project's stored stack changed; a loaded project applies it.
    StackChanged {
        /// The project whose stack changed.
        project: ProjectRoot,
    },
    /// The stored settings changed; later launches use them.
    SettingsChanged,
    /// Exits so a newer service can take over; every process keeps running.
    Replace,
    /// Exits; every process keeps running.
    Stop,
}
