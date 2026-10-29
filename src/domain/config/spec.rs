use std::path::PathBuf;

use getset::{Getters, WithSetters};
use serde::{Deserialize, Serialize};
use typed_builder::TypedBuilder;

use crate::domain::{
    config::ConfigError,
    process::{AgentTool, Process, ProcessKind, RestartPolicy, StopPolicy},
    value::{CommandLine, Description, PaneId, ProcessName},
};

/// Declarative definition of one process within a workspace section.
#[derive(Clone, Debug, Serialize, Deserialize, Getters, WithSetters, TypedBuilder)]
#[serde(try_from = "SpecFields")]
#[set_with]
pub struct ProcessSpec {
    #[getset(get = "pub", set_with = "pub")]
    name: ProcessName,
    #[getset(get = "pub", set_with = "pub")]
    #[builder(default)]
    command: Option<CommandLine>,
    #[getset(get = "pub", set_with = "pub")]
    #[builder(default)]
    working_dir: Option<PathBuf>,
    #[getset(get = "pub", set_with = "pub")]
    #[builder(default)]
    description: Option<Description>,
    #[getset(get = "pub", set_with = "pub")]
    #[builder(default)]
    restart: Option<RestartPolicy>,
    /// Optional graceful shutdown policy. Only command specs may configure it.
    #[getset(get = "pub", set_with = "pub")]
    #[builder(default)]
    stop: Option<StopPolicy>,
    /// Whether `muster up` starts this process.
    #[getset(get = "pub", set_with = "pub")]
    autostart: bool,
}

/// A process entry as written in `muster.yml`, before the rules serde cannot
/// state are checked. `autostart` is read as optional only so a missing one is
/// refused with the process's name rather than a bare position.
#[derive(Deserialize)]
struct SpecFields {
    name: ProcessName,
    command: Option<CommandLine>,
    working_dir: Option<PathBuf>,
    description: Option<Description>,
    restart: Option<RestartPolicy>,
    stop: Option<StopPolicy>,
    autostart: Option<bool>,
}

impl TryFrom<SpecFields> for ProcessSpec {
    type Error = ConfigError;

    /// Accepts an entry that states its `autostart`.
    ///
    /// # Errors
    /// Returns [`ConfigError::AutostartMissing`] naming the process otherwise.
    fn try_from(fields: SpecFields) -> Result<Self, ConfigError> {
        let SpecFields {
            name,
            command,
            working_dir,
            description,
            restart,
            stop,
            autostart,
        } = fields;
        let Some(autostart) = autostart else {
            return Err(ConfigError::AutostartMissing { name });
        };
        Ok(Self {
            name,
            command,
            working_dir,
            description,
            restart,
            stop,
            autostart,
        })
    }
}

impl ProcessSpec {
    /// Builds the corresponding `Process` entity in its initial `Pending` state.
    pub fn to_process(&self, id: PaneId, kind: ProcessKind) -> Process {
        Process::builder()
            .id(id)
            .name(self.name.clone())
            .kind(kind)
            .agent_tool(
                (kind == ProcessKind::Agent)
                    .then(|| AgentTool::from_command(self.command.as_ref())),
            )
            .command(self.command.clone())
            .working_dir(self.working_dir.clone())
            .description(self.description.clone())
            .restart(self.restart_policy())
            .stop(if kind == ProcessKind::Command {
                self.stop.clone()
            } else {
                None
            })
            .autostart(self.autostart)
            .build()
    }

    /// The effective restart policy, treating an absent policy as `Never`.
    pub fn restart_policy(&self) -> RestartPolicy {
        self.restart.unwrap_or(RestartPolicy::Never)
    }

    /// The effective graceful shutdown policy for `kind`. Commands use the
    /// domain default when the config omits `stop`; other kinds have no policy.
    pub fn effective_stop_policy(&self, kind: ProcessKind) -> Option<StopPolicy> {
        if kind == ProcessKind::Command {
            Some(self.stop.clone().unwrap_or_default())
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::config::WorkspaceConfig;

    #[test]
    fn config_without_autostart_on_a_process_fails_to_load_with_the_process_named() {
        let raw = "agents: []\nterminals: []\ncommands:\n  - name: dev server\n    command: \
                   npm run dev\n";

        let error = serde_yaml_ng::from_str::<WorkspaceConfig>(raw).unwrap_err();

        let message = error.to_string();
        assert!(message.contains("'dev server'"), "{message}");
        assert!(message.contains("autostart"), "{message}");
    }

    #[test]
    fn a_stated_autostart_is_carried_to_the_process() {
        let raw = "name: worker\ncommand: sleep 1\nautostart: true\n";

        let spec: ProcessSpec = serde_yaml_ng::from_str(raw).unwrap();

        assert!(*spec.autostart());
        assert!(
            *spec
                .to_process(PaneId::new(0), ProcessKind::Command)
                .autostart()
        );
    }
}
