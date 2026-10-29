use std::{collections::BTreeMap, path::PathBuf};

use getset::Getters;
use schemars::JsonSchema;
use serde::{
    Deserialize, Deserializer,
    de::{DeserializeOwned, Error as DeError},
};
use serde_json::{Map, Value};
use typed_builder::TypedBuilder;

use super::{
    process_fields::{
        DESCRIPTION_FIELD, ENVIRONMENT_SET_FIELD, ENVIRONMENT_UNSET_FIELD, RESTART_FIELD,
        STOP_GRACE_FIELD, STOP_SIGNAL_FIELD, WORKING_DIR_FIELD, choice, command, description,
        env_name, env_vars, stop_grace, working_dir,
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

/// The field naming the project.
const PROJECT_FIELD: &str = "project";
/// The field naming the process.
const NAME_FIELD: &str = "name";
/// The field holding the new command.
const COMMAND_FIELD: &str = "command";
/// The field saying whether `muster up` starts it.
const AUTOSTART_FIELD: &str = "autostart";
/// The field holding the readiness check.
const READY_FIELD: &str = "ready";
/// Every field the arguments may hold.
const FIELDS: &[&str] = &[
    PROJECT_FIELD,
    NAME_FIELD,
    COMMAND_FIELD,
    WORKING_DIR_FIELD,
    DESCRIPTION_FIELD,
    ENVIRONMENT_SET_FIELD,
    ENVIRONMENT_UNSET_FIELD,
    AUTOSTART_FIELD,
    RESTART_FIELD,
    STOP_SIGNAL_FIELD,
    STOP_GRACE_FIELD,
    READY_FIELD,
];

/// The changes to make to one process of the project's stack. Fields left
/// out keep their value; `working_dir`, `description` and `ready` also take
/// `null`, which clears them, so they are read by hand.
#[derive(Debug, JsonSchema, Getters, TypedBuilder)]
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
    #[builder(default)]
    working_dir: Option<Option<String>>,
    /// A one-line note about it; null removes it.
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
    #[builder(default)]
    ready: Option<Option<ReadyArg>>,
}

impl<'de> Deserialize<'de> for ProcessSetArg {
    /// Reads the arguments object field by field: an absent field keeps its
    /// value, and a nullable one given as `null` clears it. A field this
    /// struct does not name is refused.
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let mut fields = Map::<String, Value>::deserialize(deserializer)?;
        let arg = Self {
            project: given(&mut fields, PROJECT_FIELD)?,
            name: given(&mut fields, NAME_FIELD)?
                .ok_or_else(|| D::Error::missing_field(NAME_FIELD))?,
            command: given(&mut fields, COMMAND_FIELD)?,
            working_dir: nullable(&mut fields, WORKING_DIR_FIELD)?,
            description: nullable(&mut fields, DESCRIPTION_FIELD)?,
            environment_set: given(&mut fields, ENVIRONMENT_SET_FIELD)?,
            environment_unset: given(&mut fields, ENVIRONMENT_UNSET_FIELD)?,
            autostart: given(&mut fields, AUTOSTART_FIELD)?,
            restart: given(&mut fields, RESTART_FIELD)?,
            stop_signal: given(&mut fields, STOP_SIGNAL_FIELD)?,
            stop_grace: given(&mut fields, STOP_GRACE_FIELD)?,
            ready: nullable(&mut fields, READY_FIELD)?,
        };
        match fields.keys().next() {
            Some(unknown) => Err(D::Error::unknown_field(unknown, FIELDS)),
            None => Ok(arg),
        }
    }
}

/// The value of `field`, taken out of `fields`: none when it is absent or
/// `null`.
///
/// # Errors
/// Returns the error of a value that is not a `T`.
fn given<T: DeserializeOwned, E: DeError>(
    fields: &mut Map<String, Value>,
    field: &'static str,
) -> Result<Option<T>, E> {
    Ok(nullable(fields, field)?.flatten())
}

/// The value of `field`, taken out of `fields`: none when it is absent, and
/// `Some(None)` when it is `null`.
///
/// # Errors
/// Returns the error of a value that is not a `T`.
fn nullable<T: DeserializeOwned, E: DeError>(
    fields: &mut Map<String, Value>,
    field: &'static str,
) -> Result<Option<Option<T>>, E> {
    fields
        .remove(field)
        .map(|value| {
            serde_json::from_value(value).map_err(|error| E::custom(format!("{field}: {error}")))
        })
        .transpose()
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
    use serde_json::Value;

    use super::*;

    /// The arguments an agent sends as `json`.
    fn arg(json: Value) -> ProcessSetArg {
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

        assert!(refused.unwrap_err().to_string().contains("cwd"));
    }

    /// The name is required, a value of the wrong type is refused naming
    /// its field, and a plain field given as null keeps its value.
    #[test]
    fn a_missing_name_and_a_mistyped_field_are_refused() {
        let nameless = serde_json::from_value::<ProcessSetArg>(serde_json::json!({}));
        let mistyped = serde_json::from_value::<ProcessSetArg>(
            serde_json::json!({ "name": "web", "autostart": "yes" }),
        );

        assert!(nameless.unwrap_err().to_string().contains(NAME_FIELD));
        assert!(mistyped.unwrap_err().to_string().contains(AUTOSTART_FIELD));
        assert_eq!(
            *arg(serde_json::json!({ "name": "web", "command": null })).command(),
            None
        );
    }
}
