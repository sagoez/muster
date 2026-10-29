use std::path::PathBuf;

use getset::Getters;
use typed_builder::TypedBuilder;

use crate::domain::{
    config::{DeclaredPorts, Environment, ProcessSpec, launch_ports},
    process::{ProcessState, ReadyCheck, RestartPolicy, StopPolicy},
    value::{CommandLine, Description, LocalPort, PaneId, ProcessName},
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
    /// The TCP ports it listens on, checked free before each launch.
    #[builder(default)]
    ports: DeclaredPorts,
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
        self.ports = spec.ports().clone();
    }

    /// The ports a launch checks are free: those declared and the readiness
    /// port, each once, ascending.
    pub fn launch_ports(&self) -> Vec<LocalPort> {
        launch_ports(&self.ports, self.ready.as_ref())
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
            .ports(self.ports.clone())
            .build()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::domain::{
        process::{ReadyProbe, StopSignal},
        value::{EnvName, EnvValue, StopGrace},
    };

    /// A definition running `command`.
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
            .ports(DeclaredPorts::of([LocalPort::try_new(5173).unwrap()]).unwrap())
            .build()
    }

    #[test]
    fn a_new_process_is_pending_and_round_trips_its_spec() {
        let original = spec("npm run dev");
        let process = original.to_process(PaneId::new(4));

        assert_eq!(*process.state(), ProcessState::Pending);
        assert_eq!(process.spec(), original);
    }

    /// The port `raw`.
    fn port(raw: u16) -> LocalPort {
        LocalPort::try_new(raw).unwrap()
    }

    #[test]
    fn launch_ports_join_declared_and_readiness_ports_once_in_order() {
        let ready = |target: &str| {
            Some(
                ReadyCheck::builder()
                    .probe(ReadyProbe::from_stored("port", target).unwrap())
                    .timeout("30s".parse().unwrap())
                    .build(),
            )
        };
        let launched = |ports: &[u16], ready: Option<ReadyCheck>| {
            spec("npm run dev")
                .with_ports(DeclaredPorts::of(ports.iter().copied().map(port)).unwrap())
                .with_ready(ready)
                .to_process(PaneId::new(4))
                .launch_ports()
        };

        assert_eq!(launched(&[5173, 3000], ready("3000")), [
            port(3000),
            port(5173)
        ]);
        assert_eq!(launched(&[5173, 3000], ready("4000")), [
            port(3000),
            port(4000),
            port(5173)
        ]);
        assert_eq!(launched(&[], ready("4000")), [port(4000)]);
        assert!(launched(&[], None).is_empty());
    }

    /// A changed definition keeps the pane and its state.
    #[test]
    fn set_spec_keeps_identity_and_state() {
        let mut process = spec("npm run dev").to_process(PaneId::new(4));
        process.set_state(ProcessState::Running);
        let changed = spec("pnpm dev").with_ports(DeclaredPorts::default());

        process.set_spec(&changed);

        assert_eq!(*process.id(), PaneId::new(4));
        assert_eq!(*process.state(), ProcessState::Running);
        assert_eq!(process.spec(), changed);
    }
}
