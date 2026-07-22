use std::path::PathBuf;

use getset::Getters;
use typed_builder::TypedBuilder;

use crate::domain::{
    process::{
        ActivityState, AgentTool, ProcessKind, ProcessOrigin, ProcessState, RestartPolicy,
        StopPolicy,
    },
    value::{CommandLine, Description, PaneId, ProcessName},
};

/// A configured process managed by the workspace: an agent, terminal, or command.
#[derive(Clone, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct Process {
    /// Stable identity for this process and its pane.
    id: PaneId,
    /// Display name shown in the sidebar.
    name: ProcessName,
    /// Section the process belongs to.
    kind: ProcessKind,
    /// Agent preset used for provider-aware activity detection.
    #[builder(default)]
    agent_tool: Option<AgentTool>,
    /// Whether the process is configured or disposable for this TUI session.
    #[builder(default)]
    origin: ProcessOrigin,
    /// Command used to launch it, or the user's login shell when absent.
    #[builder(default)]
    command: Option<CommandLine>,
    /// Working directory to launch in; inherits the workspace cwd when absent.
    #[builder(default)]
    working_dir: Option<PathBuf>,
    /// Optional secondary line under the name in the sidebar.
    #[builder(default)]
    description: Option<Description>,
    /// Current lifecycle state.
    #[builder(default)]
    state: ProcessState,
    /// Restart policy governing what happens when the child exits.
    #[builder(default)]
    restart: RestartPolicy,
    /// Optional graceful shutdown policy, valid only for command processes.
    #[builder(default)]
    stop: Option<StopPolicy>,
    /// Whether this process launches automatically when its workspace loads.
    #[builder(default = true)]
    autostart: bool,
    /// What the process appears to be doing, inferred from its terminal signals.
    #[builder(default)]
    activity: ActivityState,
}

impl Process {
    /// Transitions the process to a new lifecycle state.
    pub fn set_state(&mut self, state: ProcessState) {
        self.state = state;
    }

    /// Sets whether this process auto-starts when its workspace loads.
    pub fn set_autostart(&mut self, autostart: bool) {
        self.autostart = autostart;
    }

    /// Updates the inferred activity of the process.
    pub fn set_activity(&mut self, activity: ActivityState) {
        self.activity = activity;
    }

    /// The effective graceful shutdown policy. Commands use the domain default
    /// when their config omits an override; other process kinds have no policy.
    pub fn effective_stop_policy(&self) -> Option<StopPolicy> {
        if self.kind == ProcessKind::Command {
            Some(self.stop.clone().unwrap_or_default())
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_with_defaults_for_optional_fields() {
        let process = Process::builder()
            .id(PaneId::new(1))
            .name(ProcessName::try_new("Claude Code").unwrap())
            .kind(ProcessKind::Agent)
            .command(Some(CommandLine::try_new("claude").unwrap()))
            .build();

        assert_eq!(*process.kind(), ProcessKind::Agent);
        assert_eq!(*process.origin(), ProcessOrigin::Configured);
        assert_eq!(*process.state(), ProcessState::Pending);
        assert!(process.working_dir().is_none());
        assert!(process.description().is_none());
    }

    #[test]
    fn command_defaults_to_none() {
        let process = Process::builder()
            .id(PaneId::new(1))
            .name(ProcessName::try_new("shell").unwrap())
            .kind(ProcessKind::Terminal)
            .build();
        assert!(process.command().is_none());
    }
}
