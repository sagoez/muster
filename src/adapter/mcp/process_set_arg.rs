use std::{collections::BTreeMap, path::PathBuf};

use getset::Getters;
use schemars::JsonSchema;
use serde::Deserialize;
use typed_builder::TypedBuilder;

use super::{
    nullable::nullable,
    process_fields::{
        ENVIRONMENT_SET_FIELD, ENVIRONMENT_UNSET_FIELD, RESTART_FIELD, STOP_SIGNAL_FIELD, choice,
        command, description, env_name, env_vars, stop_grace, working_dir,
    },
    ready_arg::ReadyArg,
};
use crate::{
    adapter::mcp::tool_error::ToolError,
    domain::{
        config::{FieldEdit, ProcessChange},
        process::ReadyCheck,
        value::{Description, ProjectRoot},
    },
};

/// The changes to make to one process of the project's stack. Fields left
/// out keep their value.
#[derive(Debug, Deserialize, JsonSchema, Getters, TypedBuilder)]
#[serde(deny_unknown_fields)]
#[getset(get = "pub")]
pub struct ProcessSetArg {
    /// A registered project's name or a folder; defaults to the project
    /// containing the server's working directory.
    #[builder(default)]
    project: Option<String>,
    /// The process's name.
    name: String,
    /// The new shell command.
    #[builder(default)]
    command: Option<String>,
    /// Where it runs, relative to the project root; null runs it in the root.
    #[serde(default, deserialize_with = "nullable")]
    #[builder(default)]
    working_dir: Option<Option<String>>,
    /// A one-line note about it; null removes it.
    #[serde(default, deserialize_with = "nullable")]
    #[builder(default)]
    description: Option<Option<String>>,
    /// Environment variables to add or replace, by name.
    #[builder(default)]
    environment_set: Option<BTreeMap<String, String>>,
    /// Names of environment variables to remove.
    #[builder(default)]
    environment_unset: Option<Vec<String>>,
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
    /// A readiness check as process_add takes; null removes it.
    #[serde(default, deserialize_with = "nullable")]
    #[builder(default)]
    ready: Option<Option<ReadyArg>>,
}

impl ProcessSetArg {
    /// The change these fields describe for a process of the project at
    /// `root`.
    ///
    /// # Errors
    /// Returns the refusal of the first field that does not read as its
    /// value.
    pub fn change(&self, root: &ProjectRoot) -> Result<ProcessChange, ToolError> {
        let unset_env = self
            .environment_unset
            .iter()
            .flatten()
            .map(|name| env_name(ENVIRONMENT_UNSET_FIELD, name))
            .collect::<Result<Vec<_>, _>>()?;
        let set_env = match &self.environment_set {
            Some(vars) => env_vars(ENVIRONMENT_SET_FIELD, vars)?,
            None => Vec::new(),
        };
        Ok(ProcessChange::builder()
            .command(self.command.as_deref().map(command).transpose()?)
            .working_dir(self.working_dir_edit(root)?)
            .description(self.description_edit()?)
            .set_env(set_env)
            .unset_env(unset_env)
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
            .ready(self.ready_edit()?)
            .build())
    }

    /// The edit `working_dir` asks for; a folder that is the root clears it.
    ///
    /// # Errors
    /// Returns the refusal of a path that is not an existing directory.
    fn working_dir_edit(&self, root: &ProjectRoot) -> Result<FieldEdit<PathBuf>, ToolError> {
        Ok(match &self.working_dir {
            None => FieldEdit::Keep,
            Some(None) => FieldEdit::Clear,
            Some(Some(raw)) => match working_dir(root, raw)? {
                Some(dir) => FieldEdit::Set(dir),
                None => FieldEdit::Clear,
            },
        })
    }

    /// The edit `ready` asks for.
    ///
    /// # Errors
    /// Returns the refusal of a readiness check out of form.
    fn ready_edit(&self) -> Result<FieldEdit<ReadyCheck>, ToolError> {
        Ok(match &self.ready {
            None => FieldEdit::Keep,
            Some(None) => FieldEdit::Clear,
            Some(Some(ready)) => FieldEdit::Set(ready.check()?),
        })
    }

    /// The edit `description` asks for.
    ///
    /// # Errors
    /// Returns the refusal of a blank description.
    fn description_edit(&self) -> Result<FieldEdit<Description>, ToolError> {
        Ok(match &self.description {
            None => FieldEdit::Keep,
            Some(None) => FieldEdit::Clear,
            Some(Some(raw)) => FieldEdit::Set(description(raw)?),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The arguments an agent sends as `json`.
    fn arg(json: serde_json::Value) -> ProcessSetArg {
        serde_json::from_value(json).unwrap()
    }

    /// An absent field keeps its value, `null` clears it and a string sets it.
    #[test]
    fn absent_null_and_a_value_are_three_edits() {
        let kept = arg(serde_json::json!({ "name": "web" }));
        let cleared = arg(serde_json::json!({ "name": "web", "description": null }));
        let set = arg(serde_json::json!({ "name": "web", "description": "the dev server" }));

        assert_eq!(kept.description_edit().unwrap(), FieldEdit::Keep);
        assert_eq!(cleared.description_edit().unwrap(), FieldEdit::Clear);
        assert_eq!(
            set.description_edit().unwrap(),
            FieldEdit::Set(Description::try_new("the dev server").unwrap())
        );
    }

    /// `ready` absent keeps the check, null clears it and an object sets it.
    #[test]
    fn ready_absent_null_and_a_check_are_three_edits() {
        let kept = arg(serde_json::json!({ "name": "web" }));
        let cleared = arg(serde_json::json!({ "name": "web", "ready": null }));
        let set = arg(serde_json::json!({
            "name": "web",
            "ready": { "port": 3000, "timeout": "30s" },
        }));

        assert_eq!(kept.ready_edit().unwrap(), FieldEdit::Keep);
        assert_eq!(cleared.ready_edit().unwrap(), FieldEdit::Clear);
        assert!(matches!(set.ready_edit().unwrap(), FieldEdit::Set(_)));
    }

    /// A field the tool does not take is refused rather than ignored, so a
    /// misspelt field never reads as no change.
    #[test]
    fn an_unknown_field_is_refused() {
        let refused = serde_json::from_value::<ProcessSetArg>(
            serde_json::json!({ "name": "web", "cwd": "frontend" }),
        );

        assert!(refused.is_err());
    }
}
