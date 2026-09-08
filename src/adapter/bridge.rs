use getset::{CopyGetters, Getters};
use serde::{Deserialize, Serialize};
use typed_builder::TypedBuilder;

use crate::domain::process::{ActivityState, Process, ProcessKind, ProcessState};

/// A query an MCP client sends the running workspace over its IPC socket. This is
/// a transport contract, not a domain concept: it lives in the adapter layer so
/// the protocol can evolve (fields, framing, versioning) without touching the
/// core, and both ends - the TUI server and the MCP client - share this one shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkspaceRequest {
    /// Enumerate the running processes with their status.
    ListProcesses,
    /// Read the latest on-screen output of the process owning `pane`.
    PaneOutput {
        /// Id of the process/pane to read, as reported by [`ProcessSummary`].
        pane: u64,
    },
    /// Write `data` to the terminal of the process owning `pane`, as if the human
    /// typed it - to answer a prompt, run a command, or unblock a process.
    SendInput {
        /// Id of the process/pane to type into.
        pane: u64,
        /// Bytes to write to the pane's terminal.
        data: String,
    },
    /// Drive the lifecycle of the process owning `pane`.
    ControlProcess {
        /// Id of the process to act on.
        pane: u64,
        /// What to do to it.
        action: ControlAction,
    },
    /// Enumerate this project's durable agent sessions.
    ListSessions,
}

/// A lifecycle action an agent can request on a process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ControlAction {
    /// Start it if it is not running.
    Start,
    /// Stop it, without its restart policy respawning it.
    Stop,
    /// Restart it regardless of its restart policy.
    Restart,
}

/// The result of a [`WorkspaceRequest::ControlProcess`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ControlOutcome {
    /// The action was handed to the workspace's lifecycle. Whether it changes
    /// anything depends on the process's current state (starting an already
    /// running process does nothing), and a stop is graceful, so the effect may
    /// land after the configured grace period. Poll `process_list` to observe it.
    Requested,
    /// No process has that id.
    Unknown,
}

/// One durable agent session in this project. `tool` and `state` carry the same
/// labels the session store persists, so this mirrors what muster records rather
/// than inventing a second vocabulary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct SessionSummary {
    /// The agent's name.
    name: String,
    /// Provider that runs it (claude, codex, ...).
    tool: String,
    /// Lifecycle of the durable record (pending, open, closed).
    state: String,
    /// Whether a native conversation was captured, so the session can be resumed.
    resumable: bool,
}

/// Wire form of a process's kind. Deliberately distinct from the domain
/// [`ProcessKind`] so the JSON agents parse never shifts because a domain label
/// changed; the mapping below is exhaustive, so a new domain variant fails to
/// compile here rather than silently escaping onto the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WireKind {
    /// A CLI coding agent.
    Agent,
    /// A plain interactive shell.
    Terminal,
    /// A long-running dev command.
    Command,
}

impl From<ProcessKind> for WireKind {
    fn from(kind: ProcessKind) -> Self {
        match kind {
            ProcessKind::Agent => WireKind::Agent,
            ProcessKind::Terminal => WireKind::Terminal,
            ProcessKind::Command => WireKind::Command,
        }
    }
}

/// Wire form of a process's lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WireState {
    /// Configured but not yet started.
    Pending,
    /// Child process is alive.
    Running,
    /// Suspended.
    Paused,
    /// Shutting down.
    Stopping,
    /// Finished on its own.
    Exited,
    /// Finished abnormally.
    Crashed,
    /// Waiting to be respawned.
    Restarting,
}

impl From<ProcessState> for WireState {
    fn from(state: ProcessState) -> Self {
        match state {
            ProcessState::Pending => WireState::Pending,
            ProcessState::Running => WireState::Running,
            ProcessState::Paused => WireState::Paused,
            ProcessState::Stopping => WireState::Stopping,
            ProcessState::Exited => WireState::Exited,
            ProcessState::Crashed => WireState::Crashed,
            ProcessState::Restarting => WireState::Restarting,
        }
    }
}

/// Wire form of a process's inferred activity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WireActivity {
    /// No recent signal.
    Idle,
    /// Producing output right now.
    Working,
    /// Waiting on the user.
    AwaitingInput,
}

impl From<ActivityState> for WireActivity {
    fn from(activity: ActivityState) -> Self {
        match activity {
            ActivityState::Idle => WireActivity::Idle,
            ActivityState::Working => WireActivity::Working,
            ActivityState::AwaitingInput => WireActivity::AwaitingInput,
        }
    }
}

/// One process in a workspace snapshot: its stable id, name, kind, lifecycle
/// state, and inferred activity - everything an agent needs to reason about what
/// is running.
#[derive(
    Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Getters, CopyGetters, TypedBuilder,
)]
pub struct ProcessSummary {
    /// Stable identifier, usable as the handle for pane reads and input.
    #[getset(get_copy = "pub")]
    id: u64,
    /// The process's display name.
    #[getset(get = "pub")]
    name: String,
    /// Whether it is an agent, a terminal, or a command.
    #[getset(get_copy = "pub")]
    kind: WireKind,
    /// Lifecycle state.
    #[getset(get_copy = "pub")]
    state: WireState,
    /// Inferred activity.
    #[getset(get_copy = "pub")]
    activity: WireActivity,
}

impl ProcessSummary {
    /// Projects a domain [`Process`] onto the wire. This mapping is the seam:
    /// domain types stay free of any transport concern.
    #[must_use]
    pub fn of(process: &Process) -> Self {
        Self::builder()
            .id((*process.id()).into_inner())
            .name(process.name().as_ref().to_string())
            .kind((*process.kind()).into())
            .state((*process.state()).into())
            .activity((*process.activity()).into())
            .build()
    }
}

/// The result of a [`WorkspaceRequest::SendInput`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SendOutcome {
    /// The input was written to a live process terminal.
    Delivered,
    /// The process exists but has no live terminal (not started, or exited).
    NotRunning,
    /// No process has that id.
    Unknown,
}

/// A running workspace's reply to a [`WorkspaceRequest`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkspaceResponse {
    /// The current process roster, in sidebar order.
    Processes(Vec<ProcessSummary>),
    /// The requested pane's latest on-screen text, or `None` if no process owns
    /// that id.
    PaneOutput(Option<String>),
    /// The outcome of writing input to a pane.
    Sent(SendOutcome),
    /// The outcome of a lifecycle action.
    Controlled(ControlOutcome),
    /// This project's durable agent sessions.
    Sessions(Vec<SessionSummary>),
    /// The workspace could not answer (it is shutting down, or the request was
    /// dropped). Distinct from an empty result, so a caller never mistakes
    /// "could not ask" for "nothing is running".
    Unavailable,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A roster round-trips through JSON with all fields intact, and the wire
    /// labels are the stable kebab-case protocol values.
    #[test]
    fn a_process_roster_round_trips() {
        let response = WorkspaceResponse::Processes(vec![
            ProcessSummary::builder()
                .id(3)
                .name("api".to_string())
                .kind(WireKind::Command)
                .state(WireState::Running)
                .activity(WireActivity::AwaitingInput)
                .build(),
        ]);

        let json = serde_json::to_string(&response).unwrap();
        assert_eq!(
            serde_json::from_str::<WorkspaceResponse>(&json).unwrap(),
            response
        );
        assert!(json.contains("\"running\""));
        assert!(json.contains("\"awaiting-input\""));
        assert!(json.contains("\"command\""));
    }

    /// Every request and every response variant round-trips.
    #[test]
    fn requests_and_responses_round_trip() {
        for request in [
            WorkspaceRequest::ListProcesses,
            WorkspaceRequest::PaneOutput { pane: 7 },
            WorkspaceRequest::SendInput {
                pane: 7,
                data: "ls\r".to_string(),
            },
            WorkspaceRequest::ControlProcess {
                pane: 7,
                action: ControlAction::Restart,
            },
            WorkspaceRequest::ListSessions,
        ] {
            let json = serde_json::to_string(&request).unwrap();
            assert_eq!(
                serde_json::from_str::<WorkspaceRequest>(&json).unwrap(),
                request
            );
        }
        for response in [
            WorkspaceResponse::PaneOutput(Some("hello\nworld".to_string())),
            WorkspaceResponse::PaneOutput(None),
            WorkspaceResponse::Sent(SendOutcome::Delivered),
            WorkspaceResponse::Sent(SendOutcome::NotRunning),
            WorkspaceResponse::Sent(SendOutcome::Unknown),
            WorkspaceResponse::Controlled(ControlOutcome::Requested),
            WorkspaceResponse::Controlled(ControlOutcome::Unknown),
            WorkspaceResponse::Sessions(vec![
                SessionSummary::builder()
                    .name("Ada".to_string())
                    .tool("Claude".to_string())
                    .state("open".to_string())
                    .resumable(true)
                    .build(),
            ]),
            WorkspaceResponse::Unavailable,
        ] {
            let json = serde_json::to_string(&response).unwrap();
            assert_eq!(
                serde_json::from_str::<WorkspaceResponse>(&json).unwrap(),
                response
            );
        }
    }

    /// Domain states project onto their wire counterparts exhaustively.
    #[test]
    fn domain_states_project_onto_the_wire() {
        assert_eq!(WireState::from(ProcessState::Crashed), WireState::Crashed);
        assert_eq!(
            WireActivity::from(ActivityState::AwaitingInput),
            WireActivity::AwaitingInput
        );
        assert_eq!(WireKind::from(ProcessKind::Agent), WireKind::Agent);
    }
}
