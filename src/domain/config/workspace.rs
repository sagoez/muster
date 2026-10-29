use std::collections::HashMap;

use getset::{Getters, WithSetters};
use serde::{Deserialize, Serialize};
use typed_builder::TypedBuilder;

use crate::domain::{
    config::{ConfigError, ProcessSpec},
    process::{Process, ProcessKind},
    value::{PaneId, ProcessName},
};

/// The full workspace definition: processes grouped into the agents,
/// terminals and commands sections. Every section must be present
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
    /// configures the command-only graceful shutdown policy,
    /// [`ConfigError::AgentCommandMissing`] when an agent omits its command, or
    /// [`ConfigError::DuplicateProcessName`] when two processes of any kinds
    /// share a name.
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
        if let Some(spec) = self.agents.iter().find(|spec| spec.command().is_none()) {
            return Err(ConfigError::AgentCommandMissing {
                name: spec.name().clone(),
            });
        }
        let mut seen: HashMap<String, (ProcessKind, usize)> = HashMap::new();
        for (kind, specs) in self.sections() {
            for (index, spec) in specs.iter().enumerate() {
                let entry = index + 1;
                if let Some((first_kind, first_entry)) =
                    seen.insert(spec.name().key(), (kind, entry))
                {
                    return Err(ConfigError::DuplicateProcessName {
                        name: spec.name().clone(),
                        first_kind,
                        first_entry,
                        second_kind: kind,
                        second_entry: entry,
                    });
                }
            }
        }
        Ok(())
    }

    /// Whether a process of any section uses `name`, however its accents are
    /// encoded.
    pub fn uses_name(&self, name: &ProcessName) -> bool {
        let key = name.key();
        self.sections()
            .into_iter()
            .flat_map(|(_, specs)| specs)
            .any(|spec| spec.name().key() == key)
    }

    /// Every section with its kind, in section order.
    fn sections(&self) -> [(ProcessKind, &Vec<ProcessSpec>); 3] {
        [
            (ProcessKind::Agent, &self.agents),
            (ProcessKind::Terminal, &self.terminals),
            (ProcessKind::Command, &self.commands),
        ]
    }

    /// Flattens every section into ordered `Process` entities, assigning each a
    /// stable `PaneId` and tagging it with its section's kind.
    pub fn to_processes(&self) -> Vec<Process> {
        let mut processes = Vec::new();
        let mut next_id = 0;
        for (kind, specs) in self.sections() {
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
    use crate::domain::{
        process::{ProcessState, RestartPolicy, StopSignal},
        value::CommandLine,
    };

    /// Grace period configured by the sample command.
    const SAMPLE_STOP_GRACE: Duration = Duration::from_secs(5);
    const SAMPLE: &str = r#"
agents:
  - name: Claude Code
    command: claude
    description: Primary coding agent
    autostart: true
terminals:
  - name: Blank terminal
    command: bash
    autostart: true
commands:
  - name: npm:dev
    command: npm run dev
    autostart: false
    restart: on-failure
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
    autostart: true
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
    autostart: true
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

    /// Two agents sharing a name would collide their durable sessions, so the
    /// config is rejected rather than loaded ambiguously.
    #[test]
    fn rejects_duplicate_agent_names() {
        let invalid: WorkspaceConfig = serde_yaml_ng::from_str(
            r#"
agents:
  - name: claude
    command: claude
    autostart: true
  - name: claude
    command: claude --model opus
    autostart: true
terminals: []
commands: []
"#,
        )
        .unwrap();

        assert!(matches!(
            invalid.validate(),
            Err(ConfigError::DuplicateProcessName { name, .. }) if name.as_ref() == "claude"
        ));
    }

    /// Every process command names a process, so a name is unique across
    /// every section, and the error names both entries.
    #[test]
    fn rejects_a_name_shared_across_sections() {
        let invalid: WorkspaceConfig = serde_yaml_ng::from_str(
            r#"
agents: []
terminals:
  - name: shell
    command: bash
    autostart: true
commands:
  - name: api
    command: sleep 600
    autostart: true
  - name: shell
    command: sleep 600
    autostart: false
"#,
        )
        .unwrap();

        let error = invalid.validate().unwrap_err();

        assert_eq!(
            error.to_string(),
            "the name 'shell' is used by two processes, terminal #1 and command #2; process names must be unique within a project"
        );
    }

    /// Names that differ only in how an accent is encoded read the same, so
    /// they count as one name.
    #[test]
    fn rejects_names_equal_in_normal_form_c() {
        let config = WorkspaceConfig::builder()
            .agents(vec![])
            .terminals(vec![])
            .commands(
                ["caf\u{e9}", "cafe\u{301}"]
                    .into_iter()
                    .map(|name| {
                        ProcessSpec::builder()
                            .name(ProcessName::try_new(name).unwrap())
                            .command(Some(CommandLine::try_new("true").unwrap()))
                            .autostart(false)
                            .build()
                    })
                    .collect(),
            )
            .build();

        assert!(matches!(
            config.validate(),
            Err(ConfigError::DuplicateProcessName { .. })
        ));
    }

    /// An agent with no command is a mislabeled login shell that can neither
    /// resume nor capture a session, so the config is rejected at load.
    #[test]
    fn rejects_a_commandless_agent() {
        let invalid: WorkspaceConfig = serde_yaml_ng::from_str(
            r#"
agents:
  - name: claude
    autostart: true
terminals: []
commands: []
"#,
        )
        .unwrap();

        assert!(matches!(
            invalid.validate(),
            Err(ConfigError::AgentCommandMissing { name }) if name.as_ref() == "claude"
        ));
    }

    /// A provider agent command loads regardless of shape - a value option, a
    /// prompt, or a composition - because muster prepends resume arguments after
    /// the executable rather than appending, so no command is rejected as
    /// unresumable.
    #[test]
    fn allows_complex_provider_agent_commands() {
        let config: WorkspaceConfig = serde_yaml_ng::from_str(
            r#"
agents:
  - name: claude
    command: claude --system-prompt foo | tee agent.log
    autostart: true
terminals: []
commands: []
"#,
        )
        .unwrap();

        assert!(config.validate().is_ok());
    }
}
