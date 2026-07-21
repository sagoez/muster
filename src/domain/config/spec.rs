use std::path::PathBuf;

use getset::{Getters, WithSetters};
use serde::{Deserialize, Serialize};
use typed_builder::TypedBuilder;

use crate::domain::{
    process::{Process, ProcessKind, RestartPolicy},
    value::{CommandLine, Description, PaneId, ProcessName},
};

/// Declarative definition of one process within a workspace section.
#[derive(Clone, Debug, Serialize, Deserialize, Getters, WithSetters, TypedBuilder)]
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
    /// Whether to launch this process automatically on load. Absent means the
    /// per-kind default; `true`/`false` overrides it.
    #[getset(get = "pub", set_with = "pub")]
    #[builder(default)]
    autostart: Option<bool>,
}

impl ProcessSpec {
    /// Builds the corresponding `Process` entity in its initial `Pending` state.
    pub fn to_process(&self, id: PaneId, kind: ProcessKind) -> Process {
        Process::builder()
            .id(id)
            .name(self.name.clone())
            .kind(kind)
            .command(self.command.clone())
            .working_dir(self.working_dir.clone())
            .description(self.description.clone())
            .restart(self.restart_policy())
            .autostart(self.should_autostart(kind))
            .build()
    }

    /// The effective restart policy, treating an absent policy as `Never`.
    pub fn restart_policy(&self) -> RestartPolicy {
        self.restart.unwrap_or(RestartPolicy::Never)
    }

    /// Whether this process auto-starts. Defaults to false for commands (which
    /// wait for an explicit start) and true for agents and terminals.
    pub fn should_autostart(&self, kind: ProcessKind) -> bool {
        self.autostart.unwrap_or(kind != ProcessKind::Command)
    }
}
