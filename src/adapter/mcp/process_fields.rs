//! Reads the fields an agent passes to the stack-editing tools into domain
//! values, naming the field and the accepted form on a refusal.

use std::{
    collections::BTreeMap,
    fmt::Display,
    fs,
    path::{Path, PathBuf},
    str::FromStr,
};

use strum::IntoEnumIterator;

use super::tool_error::ToolError;
use crate::domain::value::{
    CommandLine, Description, EnvName, EnvValue, ProcessName, ProjectRoot, StopGrace,
    ValueParseError, parse_choice,
};

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
/// The form a working directory is written in.
const WORKING_DIR_FORM: &str = "an existing directory inside the project root";
/// The form a description is written in.
const DESCRIPTION_FORM: &str = "non-blank text";
/// The form an environment variable name is written in.
const ENV_NAME_FORM: &str = "a non-empty name without '=' that does not start with MUSTER_";
/// The form an environment variable value is written in.
const ENV_VALUE_FORM: &str = "text without NUL bytes";

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
        name: raw.to_string(),
        problem: error.into(),
    })
}

/// The command an agent typed as `raw`.
///
/// # Errors
/// Returns [`ToolError::EmptyCommand`] for a blank command.
pub(super) fn command(raw: &str) -> Result<CommandLine, ToolError> {
    CommandLine::try_new(raw.to_string()).map_err(|_| ToolError::EmptyCommand)
}

/// The description an agent typed as `raw`.
///
/// # Errors
/// Returns the refusal of a blank description.
pub(super) fn description(raw: &str) -> Result<Description, ToolError> {
    Description::try_new(raw.to_string())
        .map_err(|_| invalid(DESCRIPTION_FIELD, DESCRIPTION_FORM, raw))
}

/// The environment variable name an agent typed as `raw` in `field`.
///
/// # Errors
/// Returns the refusal of a name the environment cannot hold or muster
/// reserves.
pub(super) fn env_name(field: &'static str, raw: &str) -> Result<EnvName, ToolError> {
    EnvName::try_new(raw.to_string()).map_err(|_| invalid(field, ENV_NAME_FORM, raw))
}

/// The variables an agent passed as `vars` in `field`, in name order.
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

/// The stop grace an agent typed as `raw`, a humantime duration.
///
/// # Errors
/// Returns the refusal of unreadable text, zero, or more than an hour.
pub(super) fn stop_grace(raw: &str) -> Result<StopGrace, ToolError> {
    raw.parse().map_err(|reason| ToolError::InvalidField {
        field: STOP_GRACE_FIELD,
        reason,
    })
}

/// Where a process of the project at `root` runs when an agent typed `raw`:
/// a directory under the root is stored relative to it, and the root itself
/// as none. Symlinks are resolved first, so a link out of the project is
/// refused like the folder it leads to.
///
/// # Errors
/// Returns the refusal of a path that is not an existing directory, or of
/// one outside the project root.
pub(super) fn working_dir(root: &ProjectRoot, raw: &str) -> Result<Option<PathBuf>, ToolError> {
    let typed = Path::new(raw);
    let joined = if typed.is_absolute() {
        typed.to_path_buf()
    } else {
        root.as_ref().join(typed)
    };
    let dir = fs::canonicalize(&joined)
        .ok()
        .filter(|dir| dir.is_dir())
        .ok_or_else(|| invalid(WORKING_DIR_FIELD, WORKING_DIR_FORM, raw))?;
    let base = fs::canonicalize(root.as_ref()).unwrap_or_else(|_| root.as_ref().clone());
    match dir.strip_prefix(&base) {
        Ok(inside) if inside.as_os_str().is_empty() => Ok(None),
        Ok(inside) => Ok(Some(inside.to_path_buf())),
        Err(_) => Err(ToolError::WorkingDirOutsideProject {
            dir,
            root: root.as_ref().clone(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, os::unix::fs::symlink};

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

    /// A grace period reads as a humantime duration and refuses zero.
    #[test]
    fn a_stop_grace_must_be_positive() {
        assert_eq!(stop_grace("10s").unwrap().to_string(), "10s");
        assert!(stop_grace("0s").is_err());
        assert!(matches!(
            stop_grace("2h"),
            Err(ToolError::InvalidField {
                field: STOP_GRACE_FIELD,
                ..
            })
        ));
        assert!(stop_grace("300000000y").is_err());
    }
}
