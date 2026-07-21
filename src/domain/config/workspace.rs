use getset::{Getters, WithSetters};
use serde::{Deserialize, Serialize};
use typed_builder::TypedBuilder;

use crate::domain::{
    config::{ConfigError, ProcessSpec},
    process::{Process, ProcessKind},
    value::PaneId,
};

/// The full workspace definition: processes grouped by section, matching the
/// sidebar's AGENTS / TERMINALS / COMMANDS layout. Every section must be present
/// in the file; use an empty list for a section with no processes.
#[derive(Clone, Debug, Serialize, Deserialize, Getters, WithSetters, TypedBuilder)]
#[set_with]
pub struct WorkspaceConfig {
    #[getset(get = "pub", set_with = "pub")]
    agents: Vec<ProcessSpec>,
    #[getset(get = "pub", set_with = "pub")]
    terminals: Vec<ProcessSpec>,
    #[getset(get = "pub", set_with = "pub")]
    commands: Vec<ProcessSpec>,
}

impl WorkspaceConfig {
    /// Validates cross-field rules that Serde cannot express.
    ///
    /// # Errors
    /// Returns [`ConfigError::InvalidStopPolicy`] when an agent or terminal
    /// configures the command-only graceful shutdown policy.
    pub fn validate(&self) -> Result<(), ConfigError> {
        for (kind, specs) in [
            (ProcessKind::Agent, &self.agents),
            (ProcessKind::Terminal, &self.terminals),
        ] {
            if let Some(spec) = specs.iter().find(|spec| spec.stop().is_some()) {
                return Err(ConfigError::InvalidStopPolicy {
                    kind,
                    name: spec.name().clone(),
                });
            }
        }
        Ok(())
    }

    /// Flattens every section into ordered `Process` entities, assigning each a
    /// stable `PaneId` and tagging it with its section's kind.
    pub fn to_processes(&self) -> Vec<Process> {
        let sections = [
            (ProcessKind::Agent, &self.agents),
            (ProcessKind::Terminal, &self.terminals),
            (ProcessKind::Command, &self.commands),
        ];
        let mut processes = Vec::new();
        let mut next_id = 0;
        for (kind, specs) in sections {
            for spec in specs {
                processes.push(spec.to_process(PaneId::new(next_id), kind));
                next_id += 1;
            }
        }
        processes
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::domain::process::{ProcessState, RestartPolicy, StopSignal};

    /// Grace period configured by the sample command.
    const SAMPLE_STOP_GRACE: Duration = Duration::from_secs(5);
    const SAMPLE: &str = r#"
agents:
  - name: Claude Code
    command: claude
    description: Primary coding agent
terminals:
  - name: Blank terminal
    command: bash
commands:
  - name: npm:dev
    command: npm run dev
    restart: on_failure
    stop:
      signal: interrupt
      grace_period: 5s
"#;

    #[test]
    fn parses_sections_and_flattens_to_processes() {
        let config: WorkspaceConfig = serde_yaml_ng::from_str(SAMPLE).unwrap();
        let processes = config.to_processes();

        assert_eq!(processes.len(), 3);
        assert_eq!(*processes[0].kind(), ProcessKind::Agent);
        assert_eq!(*processes[1].kind(), ProcessKind::Terminal);
        assert_eq!(*processes[2].kind(), ProcessKind::Command);
        assert_eq!(processes[0].name().as_ref(), "Claude Code");
        assert_eq!(*processes[0].state(), ProcessState::Pending);
        let stop = processes[2].stop().as_ref().unwrap();
        assert_eq!(*stop.signal(), StopSignal::Interrupt);
        assert_eq!(*stop.grace_period(), SAMPLE_STOP_GRACE);
    }

    #[test]
    fn rejects_empty_process_name() {
        let bad = r#"
agents:
  - name: "  "
    command: claude
terminals: []
commands: []
"#;
        assert!(serde_yaml_ng::from_str::<WorkspaceConfig>(bad).is_err());
    }

    #[test]
    fn absent_restart_defaults_to_never() {
        let config: WorkspaceConfig = serde_yaml_ng::from_str(SAMPLE).unwrap();
        assert_eq!(config.agents()[0].restart_policy(), RestartPolicy::Never);
        assert_eq!(
            config.commands()[0].restart_policy(),
            RestartPolicy::OnFailure
        );
    }

    /// Agents and terminals cannot opt into command-only shutdown behavior.
    #[test]
    fn rejects_a_stop_policy_outside_commands() {
        let invalid: WorkspaceConfig = serde_yaml_ng::from_str(
            r#"
agents:
  - name: Claude
    stop:
      signal: terminate
      grace_period: 5s
terminals: []
commands: []
"#,
        )
        .unwrap();

        assert!(matches!(
            invalid.validate(),
            Err(ConfigError::InvalidStopPolicy {
                kind: ProcessKind::Agent,
                ..
            })
        ));
    }
}
