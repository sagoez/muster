use getset::{Getters, WithSetters};
use serde::{Deserialize, Serialize};
use typed_builder::TypedBuilder;

use crate::domain::{
    config::ProcessSpec,
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
    use super::*;
    use crate::domain::process::{ProcessState, RestartPolicy};

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
}
