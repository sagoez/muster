use std::path::PathBuf;

use getset::Getters;
use typed_builder::TypedBuilder;

use crate::domain::{
    config::{Environment, ProcessSpec},
    process::{ProcessState, ReadyCheck, RestartPolicy, StopPolicy},
    value::{CommandLine, Description, PaneId, ProcessName},
};

/// A process managed by a project's supervisor: its definition plus its pane
/// identity and lifecycle state.
#[derive(Clone, Debug, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct Process {
    /// Stable identity for this process and its pane.
    id: PaneId,
    /// Name the CLI and MCP address the process by.
    name: ProcessName,
    /// Shell command used to launch it.
    command: CommandLine,
    /// Working directory: the project root when absent, relative to it otherwise.
    #[builder(default)]
    working_dir: Option<PathBuf>,
    /// Optional one-line description.
    #[builder(default)]
    description: Option<Description>,
    /// Environment exported to the process on top of the inherited one.
    environment: Environment,
    /// Current lifecycle state.
    #[builder(default)]
    state: ProcessState,
    /// Restart policy governing what happens when the child exits.
    restart: RestartPolicy,
    /// Graceful shutdown policy.
    stop: StopPolicy,
    /// Whether `muster up` starts this process.
    autostart: bool,
    /// What each run waits for before it counts as ready, when anything.
    #[builder(default)]
    ready: Option<ReadyCheck>,
}

impl Process {
    /// Transitions the process to a new lifecycle state.
    pub fn set_state(&mut self, state: ProcessState) {
        self.state = state;
    }

    /// Takes `spec`'s definition in place, keeping this pane and its state.
    pub fn set_spec(&mut self, spec: &ProcessSpec) {
        self.name = spec.name().clone();
        self.command = spec.command().clone();
        self.working_dir = spec.working_dir().clone();
        self.description = spec.description().clone();
        self.environment = spec.environment().clone();
        self.restart = *spec.restart();
        self.stop = *spec.stop();
        self.autostart = *spec.autostart();
        self.ready = spec.ready().clone();
    }

    /// The definition this process runs, without its pane or state.
    pub fn spec(&self) -> ProcessSpec {
        ProcessSpec::builder()
            .name(self.name.clone())
            .command(self.command.clone())
            .working_dir(self.working_dir.clone())
            .description(self.description.clone())
            .environment(self.environment.clone())
            .autostart(self.autostart)
            .restart(self.restart)
            .stop(self.stop)
            .ready(self.ready.clone())
            .build()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::domain::{
        process::StopSignal,
        value::{EnvName, EnvValue, StopGrace},
    };

    fn spec(command: &str) -> ProcessSpec {
        ProcessSpec::builder()
            .name(ProcessName::try_new("web").unwrap())
            .command(CommandLine::try_new(command).unwrap())
            .working_dir(Some(PathBuf::from("frontend")))
            .description(None)
            .environment(Environment::empty().with_var(
                EnvName::try_new("PORT").unwrap(),
                EnvValue::try_new("5173").unwrap(),
            ))
            .autostart(true)
            .restart(RestartPolicy::OnFailure)
            .stop(
                StopPolicy::builder()
                    .signal(StopSignal::Interrupt)
                    .grace_period(StopGrace::try_new(Duration::from_secs(3)).unwrap())
                    .build(),
            )
            .build()
    }

    #[test]
    fn a_new_process_is_pending_and_round_trips_its_spec() {
        let original = spec("npm run dev");
        let process = original.to_process(PaneId::new(4));

        assert_eq!(*process.state(), ProcessState::Pending);
        assert_eq!(process.spec(), original);
    }

    /// A changed definition keeps the pane and its state.
    #[test]
    fn set_spec_keeps_identity_and_state() {
        let mut process = spec("npm run dev").to_process(PaneId::new(4));
        process.set_state(ProcessState::Running);
        let changed = spec("pnpm dev");

        process.set_spec(&changed);

        assert_eq!(*process.id(), PaneId::new(4));
        assert_eq!(*process.state(), ProcessState::Running);
        assert_eq!(process.spec(), changed);
    }
}
