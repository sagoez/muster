use std::collections::BTreeMap;

use getset::Getters;
use schemars::JsonSchema;
use serde::Deserialize;
use typed_builder::TypedBuilder;

use super::{
    process_fields::{
        ENVIRONMENT_FIELD, RESTART_FIELD, STOP_SIGNAL_FIELD, choice, command, description,
        env_vars, process_name, stop_grace, working_dir,
    },
    ready_arg::ReadyArg,
};
use crate::{
    adapter::mcp::tool_error::ToolError,
    domain::{
        config::{Environment, ProcessDraft},
        value::ProjectRoot,
    },
};

/// A process to add to the project's stack. Fields left out take the
/// `default_*` settings.
#[derive(Debug, Deserialize, JsonSchema, Getters, TypedBuilder)]
#[serde(deny_unknown_fields)]
#[getset(get = "pub")]
pub struct ProcessAddArg {
    /// A registered project's name or a folder; defaults to the project
    /// containing the server's working directory, or that directory itself,
    /// which this registers.
    #[builder(default)]
    project: Option<String>,
    /// The process's name, unique in this project.
    name: String,
    /// The shell command it runs.
    command: String,
    /// Where it runs, relative to the project root. Defaults to the root.
    #[builder(default)]
    working_dir: Option<String>,
    /// A one-line note about it.
    #[builder(default)]
    description: Option<String>,
    /// Environment variables it runs with, by name.
    #[builder(default)]
    environment: Option<BTreeMap<String, String>>,
    /// Whether `muster up` starts it.
    #[builder(default)]
    autostart: Option<bool>,
    /// `never`, `on-failure` or `always`.
    #[builder(default)]
    restart: Option<String>,
    /// `terminate` (SIGTERM) or `interrupt` (SIGINT).
    #[builder(default)]
    stop_signal: Option<String>,
    /// How long a stop waits before it kills, such as `5s`.
    #[builder(default)]
    stop_grace: Option<String>,
    /// A readiness check such as {"log": "Listening on", "timeout": "60s"},
    /// {"port": 3000, "timeout": "30s"} or {"http":
    /// "http://localhost:3000/health", "timeout": "30s"}.
    #[builder(default)]
    ready: Option<ReadyArg>,
}

impl ProcessAddArg {
    /// The draft these fields describe for the project at `root`.
    ///
    /// # Errors
    /// Returns the refusal of the first field that does not read as its
    /// value.
    pub fn draft(&self, root: &ProjectRoot) -> Result<ProcessDraft, ToolError> {
        let environment = match &self.environment {
            Some(vars) => env_vars(ENVIRONMENT_FIELD, vars)?.into_iter().collect(),
            None => Environment::empty(),
        };
        Ok(ProcessDraft::builder()
            .name(process_name(&self.name)?)
            .command(command(&self.command)?)
            .working_dir(match &self.working_dir {
                Some(raw) => working_dir(root, raw)?,
                None => None,
            })
            .description(self.description.as_deref().map(description).transpose()?)
            .environment(environment)
            .autostart(self.autostart)
            .restart(
                self.restart
                    .as_deref()
                    .map(|raw| choice(RESTART_FIELD, raw))
                    .transpose()?,
            )
            .stop_signal(
                self.stop_signal
                    .as_deref()
                    .map(|raw| choice(STOP_SIGNAL_FIELD, raw))
                    .transpose()?,
            )
            .stop_grace(self.stop_grace.as_deref().map(stop_grace).transpose()?)
            .build())
    }
}
