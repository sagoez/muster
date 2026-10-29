use std::{
    collections::BTreeMap,
    path::{Component, PathBuf},
};

use getset::Getters;
use serde::{Deserialize, Serialize};
use typed_builder::TypedBuilder;

use super::{EntryError, ReadyEntry, StopEntry};
use crate::domain::{
    config::{Environment, ProcessSpec},
    process::{ReadyCheck, RestartPolicy, StopPolicy},
    value::{
        CommandLine, Description, EnvName, EnvValue, ProcessName, ProcessNameError, ProjectRoot,
        StopGrace,
    },
};

/// One process of a stack file. Optional keys are written only when set, and
/// a variable whose value is `None` is the export's placeholder.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Getters, TypedBuilder)]
#[serde(deny_unknown_fields)]
#[getset(get = "pub")]
pub struct ProcessEntry {
    /// The process's name.
    name: String,
    /// The shell command it runs.
    command: String,
    /// Its working directory, relative to the project root and under it.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[builder(default)]
    working_dir: Option<PathBuf>,
    /// Its one-line description.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[builder(default)]
    description: Option<String>,
    /// Its variables, sorted by name; a null value is an unfilled placeholder.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[builder(default)]
    environment: Option<BTreeMap<String, Option<String>>>,
    /// Whether `muster up` starts it.
    autostart: bool,
    /// Its restart policy.
    restart: RestartPolicy,
    /// How it is stopped.
    stop: StopEntry,
    /// What each run waits for before it counts as ready.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[builder(default)]
    ready: Option<ReadyEntry>,
}

impl ProcessEntry {
    /// This entry with its working directory written as an import of it into
    /// `root` reads it: relative to `root`, and left out when it is `root`.
    ///
    /// # Errors
    /// Returns the [`EntryError`] an import would refuse the directory with,
    /// such as one outside `root`.
    pub fn relative_to(self, root: &ProjectRoot) -> Result<Self, EntryError> {
        let working_dir = match self.working_dir {
            Some(dir) if dir.is_absolute() => match root.relative(&dir) {
                Some(inside) if inside.as_os_str().is_empty() => None,
                Some(inside) => Some(working_dir(inside)?),
                None => return Err(EntryError::WorkingDirOutsideRoot { dir }),
            },
            Some(dir) => Some(working_dir(dir)?),
            None => None,
        };
        Ok(Self {
            working_dir,
            ..self
        })
    }

    /// This entry with every variable's value replaced by the placeholder.
    pub fn without_env_values(self) -> Self {
        Self {
            environment: self
                .environment
                .map(|vars| vars.into_keys().map(|name| (name, None)).collect()),
            ..self
        }
    }
}

impl From<&ProcessSpec> for ProcessEntry {
    /// The entry exporting `spec`, values included.
    fn from(spec: &ProcessSpec) -> Self {
        let vars = spec.environment().vars();
        Self {
            name: spec.name().to_string(),
            command: spec.command().to_string(),
            working_dir: spec.working_dir().clone(),
            description: spec.description().as_ref().map(ToString::to_string),
            environment: (!vars.is_empty()).then(|| {
                vars.iter()
                    .map(|(name, value)| (name.to_string(), Some(value.to_string())))
                    .collect()
            }),
            autostart: *spec.autostart(),
            restart: *spec.restart(),
            stop: StopEntry::builder()
                .signal(*spec.stop().signal())
                .grace_period(spec.stop().grace_period().into_inner())
                .build(),
            ready: spec.ready().as_ref().map(ReadyEntry::from),
        }
    }
}

/// Reads the entry's name, refusing a blank one and one holding a control
/// character each for its own reason.
fn process_name(raw: String) -> Result<ProcessName, EntryError> {
    ProcessName::try_new(raw).map_err(|error| match error {
        ProcessNameError::NotEmptyViolated => EntryError::EmptyName,
        ProcessNameError::PredicateViolated => EntryError::ControlCharacterName,
    })
}

/// Reads the entry's working directory: a non-empty folder given relative to
/// the project root that never climbs above it.
fn working_dir(dir: PathBuf) -> Result<PathBuf, EntryError> {
    if dir.as_os_str().is_empty() {
        return Err(EntryError::EmptyWorkingDir);
    }
    let mut depth: usize = 0;
    for component in dir.components() {
        depth = match component {
            Component::Normal(_) => depth + 1,
            Component::CurDir => depth,
            Component::ParentDir => match depth.checked_sub(1) {
                Some(depth) => depth,
                None => return Err(EntryError::WorkingDirOutsideRoot { dir }),
            },
            Component::RootDir | Component::Prefix(_) => {
                return Err(EntryError::WorkingDirOutsideRoot { dir });
            },
        };
    }
    Ok(dir)
}

/// Reads one variable, refusing an unfilled placeholder.
fn variable(raw: String, value: Option<String>) -> Result<(EnvName, EnvValue), EntryError> {
    let name = EnvName::try_new(raw.clone())
        .map_err(|_| EntryError::InvalidEnvName { name: raw.clone() })?;
    let value = value.ok_or_else(|| EntryError::MissingEnvValue { name: raw.clone() })?;
    let value = EnvValue::try_new(value).map_err(|_| EntryError::InvalidEnvValue { name: raw })?;
    Ok((name, value))
}

impl TryFrom<ProcessEntry> for ProcessSpec {
    type Error = EntryError;

    /// The definition `entry` describes.
    ///
    /// # Errors
    /// Returns the [`EntryError`] naming the first invalid field.
    fn try_from(entry: ProcessEntry) -> Result<Self, Self::Error> {
        let environment = entry
            .environment
            .unwrap_or_default()
            .into_iter()
            .map(|(name, value)| variable(name, value))
            .collect::<Result<Environment, EntryError>>()?;
        Ok(ProcessSpec::builder()
            .name(process_name(entry.name)?)
            .command(CommandLine::try_new(entry.command).map_err(|_| EntryError::EmptyCommand)?)
            .working_dir(entry.working_dir.map(working_dir).transpose()?)
            .description(
                entry
                    .description
                    .map(Description::try_new)
                    .transpose()
                    .map_err(|_| EntryError::EmptyDescription)?,
            )
            .environment(environment)
            .autostart(entry.autostart)
            .restart(entry.restart)
            .stop(
                StopPolicy::builder()
                    .signal(*entry.stop.signal())
                    .grace_period(
                        StopGrace::try_new(*entry.stop.grace_period())
                            .map_err(|_| EntryError::GraceOutOfRange)?,
                    )
                    .build(),
            )
            .ready(entry.ready.map(ReadyCheck::try_from).transpose()?)
            .build())
    }
}
