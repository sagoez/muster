use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::process_action::ProcessAction;
use crate::{
    adapter::bridge::WorkspaceRequest,
    domain::{
        process::AgentTool,
        value::{CommandLine, ProcessName},
    },
};

/// One request a client sends the service, as one JSON line. A `project` is
/// the path of that project's `muster.yml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ServiceRequest {
    /// Introduces the client's build, so a client of another version can
    /// replace the service.
    Hello {
        /// The client's muster version.
        version: String,
    },
    /// Starts the project's `autostart` processes that are not running.
    Up {
        /// The project to bring up.
        project: PathBuf,
    },
    /// Stops the project's commands and terminals, and its agents when asked.
    Down {
        /// The project to bring down.
        project: PathBuf,
        /// Whether agents stop too.
        agents: bool,
    },
    /// Reports every process of one project, or of every loaded project.
    Ps {
        /// The project to report, or every loaded project when absent.
        project: Option<PathBuf>,
    },
    /// Applies a lifecycle action to one process.
    Control {
        /// The process's project.
        project: PathBuf,
        /// The process.
        name: ProcessName,
        /// What to do to it.
        action: ProcessAction,
    },
    /// Finds where a running process's holder and log are.
    Locate {
        /// The process's project.
        project: PathBuf,
        /// The process.
        name: ProcessName,
    },
    /// Finds the log every run of a process writes to, whether or not it is
    /// running.
    Log {
        /// The process's project.
        project: PathBuf,
        /// The process.
        name: ProcessName,
    },
    /// Reads a process's visible screen.
    Screen {
        /// The process's project.
        project: PathBuf,
        /// The process.
        name: ProcessName,
    },
    /// Opens and starts a new agent session.
    AgentNew {
        /// The project to open it in.
        project: PathBuf,
        /// The provider to run.
        tool: AgentTool,
        /// Its name, or a generated one when absent.
        name: Option<ProcessName>,
        /// The command that starts it, in place of the provider's default.
        command: Option<CommandLine>,
        /// The template that resumes a closed conversation.
        resume: Option<CommandLine>,
    },
    /// Closes a runtime agent session.
    AgentClose {
        /// The session's project.
        project: PathBuf,
        /// The session.
        name: ProcessName,
    },
    /// Reopens the project's most recently closed agent session.
    AgentReopen {
        /// The project whose session to reopen.
        project: PathBuf,
    },
    /// Reports the recent notices of a project and of the service.
    Notices {
        /// The project whose notices to report.
        project: PathBuf,
    },
    /// A workspace request from an MCP client.
    Workspace {
        /// The project the client serves.
        project: PathBuf,
        /// The request.
        request: WorkspaceRequest,
    },
    /// Exits so a newer service can take over; every process keeps running.
    Replace,
    /// Exits; every process keeps running.
    Stop,
}
