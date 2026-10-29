//! Reads the fields an agent passes to the stack-editing tools into domain
//! values, naming the field and the accepted form on a refusal.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Display,
    path::PathBuf,
    str::FromStr,
};

use serde_json::Number;
use strum::IntoEnumIterator;

use super::{condition_arg::PORT_FORM, seconds_arg::SecondsArg, tool_error::ToolError};
use crate::{
    adapter::client::WorkingDirPlace,
    domain::{
        config::{ConfigError, DeclaredPorts, Environment},
        value::{
            COMMAND_MAX_CHARS, CommandLine, CommandLineError, DESCRIPTION_MAX_CHARS, Description,
            DescriptionError, EnvName, EnvValue, LocalPort, MAX_STOP_GRACE, ProcessName,
            ProjectRoot,
            StopGrace, ValueParseError, parse_choice, refused_name,
        },
    },
};

/// The `command` field.
pub(super) const COMMAND_FIELD: &str = "command";
/// The form a bounded text is written in, followed by its maximum.
const LENGTH_FORM: &str = "at most ";
/// The `working_dir` field.
pub(super) const WORKING_DIR_FIELD: &str = "working_dir";
/// The `description` field.
pub(super) const DESCRIPTION_FIELD: &str = "description";
/// The `environment` field of `process_add`.
pub(super) const ENVIRONMENT_FIELD: &str = "environment";
/// The `environment_set` field of `process_set`.
pub(super) const ENVIRONMENT_SET_FIELD: &str = "environment_set";
/// The `environment_unset` field of `process_set`.
pub(super) const ENVIRONMENT_UNSET_FIELD: &str = "environment_unset";
/// The `restart` field.
pub(super) const RESTART_FIELD: &str = "restart";
/// The `stop_signal` field.
pub(super) const STOP_SIGNAL_FIELD: &str = "stop_signal";
/// The `stop_grace` field.
pub(super) const STOP_GRACE_FIELD: &str = "stop_grace";
/// The `key` field of the config tools.
pub(super) const KEY_FIELD: &str = "key";
/// The form a stop grace is written in, followed by its maximum.
const STOP_GRACE_FORM: &str = "a positive number of seconds in whole milliseconds up to ";
/// The form a working directory is written in.
const WORKING_DIR_FORM: &str = "an existing directory inside the project root";
/// The form a description is written in.
const DESCRIPTION_FORM: &str = "non-blank text";
/// The form an environment variable name is written in.
const ENV_NAME_FORM: &str = "a non-empty name without '=' that does not start with MUSTER_";
/// The form an environment variable value is written in.
const ENV_VALUE_FORM: &str = "text without NUL bytes";
/// The `ports` field.
pub(super) const PORTS_FIELD: &str = "ports";
/// The form a list of declared ports is written in.
const DISTINCT_PORTS_FORM: &str = "each port once";

/// The refusal of `found` as the field `field`, which accepts `expected`.
pub(super) fn invalid(field: &'static str, expected: &str, found: &str) -> ToolError {
    ToolError::InvalidField {
        field,
        reason: ValueParseError::builder()
            .expected(expected)
            .found(found)
            .build(),
    }
}

/// The process name an agent typed as `raw`.
///
/// # Errors
/// Returns [`ToolError::InvalidName`] for a blank name or one holding a
/// control character.
pub(super) fn process_name(raw: &str) -> Result<ProcessName, ToolError> {
    ProcessName::try_new(raw.to_string()).map_err(|error| ToolError::InvalidName {
        name: refused_name(raw),
        problem: error.into(),
    })
}

/// The refusal of `text`, given as `field`, for running past `max`
/// characters. Names the length, never echoes the text.
pub(super) fn too_long(field: &'static str, max: usize, text: &str) -> ToolError {
    invalid(
        field,
        &format!("{LENGTH_FORM}{max} characters"),
        &format!("{} characters", text.chars().count()),
    )
}

/// The command an agent typed as `raw`.
///
/// # Errors
/// Returns [`ToolError::EmptyCommand`] for a blank command, or the refusal of
/// one past [`COMMAND_MAX_CHARS`].
pub(super) fn command(raw: &str) -> Result<CommandLine, ToolError> {
    CommandLine::try_new(raw.to_string()).map_err(|error| match error {
        CommandLineError::NotEmptyViolated => ToolError::EmptyCommand,
        CommandLineError::LenCharMaxViolated => {
            too_long(COMMAND_FIELD, COMMAND_MAX_CHARS, raw.trim())
        },
    })
}

/// The description an agent typed as `raw`.
///
/// # Errors
/// Returns the refusal of a blank description, or of one past
/// [`DESCRIPTION_MAX_CHARS`].
pub(super) fn description(raw: &str) -> Result<Description, ToolError> {
    Description::try_new(raw.to_string()).map_err(|error| match error {
        DescriptionError::NotEmptyViolated => invalid(DESCRIPTION_FIELD, DESCRIPTION_FORM, raw),
        DescriptionError::LenCharMaxViolated => {
            too_long(DESCRIPTION_FIELD, DESCRIPTION_MAX_CHARS, raw.trim())
        },
    })
}

/// The environment variable name an agent typed as `raw` in `field`.
///
/// # Errors
/// Returns the refusal of a name the environment cannot hold or muster
/// reserves.
pub(super) fn env_name(field: &'static str, raw: &str) -> Result<EnvName, ToolError> {
    EnvName::try_new(raw.to_string()).map_err(|_| invalid(field, ENV_NAME_FORM, raw))
}

/// The variables an agent passed as `vars` in `field`, in name order. Their
/// caps are held by the write that stores them, which names the variable.
///
/// # Errors
/// Returns the refusal of the first name or value the environment cannot
/// hold.
pub(super) fn env_vars(
    field: &'static str,
    vars: &BTreeMap<String, String>,
) -> Result<Vec<(EnvName, EnvValue)>, ToolError> {
    vars.iter()
        .map(|(name, value)| {
            let value = EnvValue::try_new(value.clone())
                .map_err(|_| invalid(field, ENV_VALUE_FORM, value))?;
            Ok((env_name(field, name)?, value))
        })
        .collect()
}

/// The environment an agent passed as `vars` in `field`.
///
/// # Errors
/// Returns the refusals of [`env_vars`].
pub(super) fn environment(
    field: &'static str,
    vars: &BTreeMap<String, String>,
) -> Result<Environment, ToolError> {
    Ok(Environment::of(env_vars(field, vars)?))
}

/// The ports an agent declared as `raw`.
///
/// # Errors
/// Returns the refusal naming [`PORTS_FIELD`] of a value that is not a port
/// from 1 to 65535, or of a port given twice, and
/// [`ConfigError::TooManyPorts`] past
/// [`DECLARED_PORTS_MAX`](crate::domain::config::DECLARED_PORTS_MAX).
pub(super) fn ports(raw: &[Number]) -> Result<DeclaredPorts, ToolError> {
    let mut declared = BTreeSet::new();
    for number in raw {
        let port = number
            .as_u64()
            .and_then(|port| u16::try_from(port).ok())
            .and_then(|port| LocalPort::try_new(port).ok())
            .ok_or_else(|| invalid(PORTS_FIELD, PORT_FORM, &number.to_string()))?;
        if !declared.insert(port) {
            return Err(invalid(
                PORTS_FIELD,
                DISTINCT_PORTS_FORM,
                &number.to_string(),
            ));
        }
    }
    Ok(DeclaredPorts::of(declared).map_err(ConfigError::from)?)
}

/// `raw` read as one of `T`'s kebab-case names in `field`.
///
/// # Errors
/// Returns the refusal listing every accepted name.
pub(super) fn choice<T>(field: &'static str, raw: &str) -> Result<T, ToolError>
where
    T: FromStr + IntoEnumIterator + Display,
{
    parse_choice(raw).map_err(|reason| ToolError::InvalidField { field, reason })
}

/// The stop grace an agent wrote as `seconds`.
///
/// # Errors
/// Returns the refusal of zero, a negative number, or more than
/// [`MAX_STOP_GRACE`].
pub(super) fn stop_grace(seconds: &SecondsArg) -> Result<StopGrace, ToolError> {
    seconds
        .duration()
        .and_then(|grace| StopGrace::try_new(grace).ok())
        .ok_or_else(|| {
            invalid(
                STOP_GRACE_FIELD,
                &format!("{STOP_GRACE_FORM}{}", MAX_STOP_GRACE.as_secs()),
                &seconds.written(),
            )
        })
}

/// Where a process of the project at `root` runs when an agent typed `raw`,
/// read relative to the root: placed as the CLI places a folder, and confined
/// to the project, so a folder outside it, or a link out of it, is refused.
///
/// # Errors
/// Returns the refusal of a path that is not an existing directory, or of
/// one outside the project root.
pub(super) fn working_dir(root: &ProjectRoot, raw: &str) -> Result<Option<PathBuf>, ToolError> {
    let place = WorkingDirPlace::of(root, &root.as_ref().join(raw))
        .map_err(|_| invalid(WORKING_DIR_FIELD, WORKING_DIR_FORM, raw))?;
    match place {
        WorkingDirPlace::Outside(dir) => Err(ToolError::WorkingDirOutsideProject {
            dir,
            root: root.as_ref().clone(),
        }),
        WorkingDirPlace::Root | WorkingDirPlace::Inside(_) => Ok(place.stored()),
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, os::unix::fs::symlink, time::Duration};

    use tempfile::TempDir;

    use super::*;
    use crate::domain::process::RestartPolicy;

    /// The canonical root of a fresh project folder with a `frontend` folder
    /// inside it, held alive by the returned directory.
    fn project() -> (TempDir, ProjectRoot) {
        let dir = TempDir::new().unwrap();
        fs::create_dir(dir.path().join("frontend")).unwrap();
        let root = ProjectRoot::try_new(fs::canonicalize(dir.path()).unwrap()).unwrap();
        (dir, root)
    }

    /// A folder under the root is stored relative to it, the root itself as
    /// none, and a missing folder is refused naming the field.
    #[test]
    fn a_working_dir_is_stored_relative_to_the_root() {
        let (_dir, root) = project();

        assert_eq!(
            working_dir(&root, "frontend").unwrap(),
            Some(PathBuf::from("frontend"))
        );
        assert_eq!(working_dir(&root, ".").unwrap(), None);
        assert_eq!(working_dir(&root, "frontend/..").unwrap(), None);
        let missing = working_dir(&root, "nowhere").unwrap_err().to_string();
        assert!(missing.starts_with("invalid working_dir:"), "{missing}");
    }

    /// A folder outside the root is refused, whether typed absolute, climbed
    /// to with `..`, or reached through a symlink inside the project; an
    /// absolute folder inside it is stored relative.
    #[test]
    fn a_working_dir_outside_the_root_is_refused() {
        let (_dir, root) = project();
        let (_other, elsewhere) = project();
        symlink(elsewhere.as_ref(), root.as_ref().join("escape")).unwrap();

        for raw in [
            elsewhere.as_ref().display().to_string(),
            "..".to_string(),
            "escape".to_string(),
            "/".to_string(),
        ] {
            assert!(
                matches!(
                    working_dir(&root, &raw),
                    Err(ToolError::WorkingDirOutsideProject { .. })
                ),
                "{raw}"
            );
        }
        assert_eq!(
            working_dir(&root, &root.as_ref().join("frontend").display().to_string()).unwrap(),
            Some(PathBuf::from("frontend"))
        );
    }

    /// An enumerated field accepts its kebab-case names and lists them on a
    /// refusal.
    #[test]
    fn a_choice_lists_the_accepted_names() {
        assert_eq!(
            choice::<RestartPolicy>(RESTART_FIELD, "on-failure").unwrap(),
            RestartPolicy::OnFailure
        );
        assert_eq!(
            choice::<RestartPolicy>(RESTART_FIELD, "sometimes")
                .unwrap_err()
                .to_string(),
            "invalid restart: expected never, on-failure, always, got 'sometimes'"
        );
    }

    /// A reserved or malformed variable name is refused naming the field.
    #[test]
    fn a_reserved_variable_name_is_refused() {
        let vars = BTreeMap::from([("MUSTER_PROJECT".to_string(), "x".to_string())]);

        let refused = env_vars(ENVIRONMENT_FIELD, &vars).unwrap_err().to_string();

        assert!(refused.starts_with("invalid environment:"), "{refused}");
        assert!(refused.contains("MUSTER_PROJECT"), "{refused}");
    }

    /// A grace period reads as seconds and refuses zero, a negative number,
    /// more than the maximum or a fraction of a millisecond, naming the rule.
    #[test]
    fn a_stop_grace_must_be_positive() {
        let seconds = |json| serde_json::from_value::<SecondsArg>(json).unwrap();

        assert_eq!(
            stop_grace(&seconds(serde_json::json!(10)))
                .unwrap()
                .to_string(),
            "10s"
        );
        assert_eq!(
            stop_grace(&seconds(serde_json::json!(0.5)))
                .unwrap()
                .to_string(),
            "500ms"
        );
        assert_eq!(
            stop_grace(&seconds(serde_json::json!(1.001)))
                .unwrap()
                .into_inner(),
            Duration::from_millis(1_001)
        );
        for refused in [0.0, -1.0, 7200.0, 1e300, 0.0004, 1.0007] {
            let refused = stop_grace(&seconds(serde_json::json!(refused))).unwrap_err();
            assert!(
                matches!(refused, ToolError::InvalidField {
                    field: STOP_GRACE_FIELD,
                    ..
                }),
                "{refused}"
            );
            let named = format!("whole milliseconds up to {}", MAX_STOP_GRACE.as_secs());
            assert!(refused.to_string().contains(&named), "{refused}");
        }
    }
}
