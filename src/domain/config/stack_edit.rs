use std::fmt::{self, Display, Formatter};

use super::working_dir::WorkingDir;
use crate::domain::{
    config::{ProcessField, ProcessSpec, SettingKey},
    coordination::Author,
    value::ProcessName,
};

/// What separates the fields an edit names.
const FIELD_SEPARATOR: &str = ", ";

/// One agent's edit to a project's stack, as the user is told of it. Its text
/// never shows an environment value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StackEdit {
    /// `author` added `spec`.
    Added {
        /// Who edited.
        author: Author,
        /// The process as stored.
        spec: ProcessSpec,
    },
    /// `author` changed `fields` of a process, leaving it as `spec`.
    Changed {
        /// Who edited.
        author: Author,
        /// The fields the edit set or cleared.
        fields: Vec<ProcessField>,
        /// The process as stored after the edit.
        spec: ProcessSpec,
    },
    /// `author` removed the process `name`.
    Removed {
        /// Who edited.
        author: Author,
        /// The process removed.
        name: ProcessName,
    },
    /// `author` set the machine-wide setting `key`, which every project's
    /// new processes take, to `value`.
    Configured {
        /// Who edited.
        author: Author,
        /// The setting.
        key: SettingKey,
        /// Its new value, written as `config get` shows it.
        value: String,
    },
}

impl StackEdit {
    /// The process the edit concerns, or none for a setting.
    pub fn process(&self) -> Option<&ProcessName> {
        match self {
            Self::Added { spec, .. } | Self::Changed { spec, .. } => Some(spec.name()),
            Self::Removed { name, .. } => Some(name),
            Self::Configured { .. } => None,
        }
    }

    /// Who made the edit.
    pub fn author(&self) -> &Author {
        match self {
            Self::Added { author, .. }
            | Self::Changed { author, .. }
            | Self::Removed { author, .. }
            | Self::Configured { author, .. } => author,
        }
    }

    /// Whether the project's edit log keeps this edit: only an agent's edits
    /// are kept, since the human made theirs at the CLI.
    pub fn is_logged(&self) -> bool {
        !self.author().is_human()
    }
}

impl Display for StackEdit {
    /// The edit for a person: who, what, and the command and working
    /// directory it now runs with whenever the edit set them.
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Added { author, spec } => write!(
                f,
                "agent {} added {}, running `{}` in {}",
                author.label(),
                spec.name().shell_quoted(),
                spec.command(),
                WorkingDir(spec.working_dir())
            ),
            Self::Changed {
                author,
                fields,
                spec,
            } => {
                let named: Vec<String> = fields.iter().map(ToString::to_string).collect();
                write!(
                    f,
                    "agent {} changed {} of {}",
                    author.label(),
                    named.join(FIELD_SEPARATOR),
                    spec.name().shell_quoted()
                )?;
                if fields.contains(&ProcessField::Command) {
                    write!(f, "; command now `{}`", spec.command())?;
                }
                if fields.contains(&ProcessField::WorkingDir) {
                    write!(f, "; working dir now {}", WorkingDir(spec.working_dir()))?;
                }
                Ok(())
            },
            Self::Removed { author, name } => write!(
                f,
                "agent {} removed {}",
                author.label(),
                name.shell_quoted()
            ),
            Self::Configured { author, key, value } => {
                write!(f, "agent {} set {key} to {value}", author.label())
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{path::PathBuf, time::Duration};

    use super::*;
    use crate::domain::{
        config::Environment,
        process::{RestartPolicy, StopPolicy, StopSignal},
        value::{CommandLine, EnvName, EnvValue, StopGrace},
    };

    fn spec() -> ProcessSpec {
        ProcessSpec::builder()
            .name(ProcessName::try_new("web").unwrap())
            .command(CommandLine::try_new("npm run dev").unwrap())
            .working_dir(Some(PathBuf::from("frontend")))
            .environment(Environment::empty().with_var(
                EnvName::try_new("TOKEN").unwrap(),
                EnvValue::try_new("s3cret").unwrap(),
            ))
            .autostart(false)
            .restart(RestartPolicy::Never)
            .stop(
                StopPolicy::builder()
                    .signal(StopSignal::Terminate)
                    .grace_period(StopGrace::try_new(Duration::from_secs(5)).unwrap())
                    .build(),
            )
            .build()
    }

    fn agent() -> Author {
        Author::agent("claude-code@builder")
    }

    /// An addition names who, the process, its command and where it runs,
    /// and never an environment value.
    #[test]
    fn an_addition_names_the_command_and_working_dir() {
        let edit = StackEdit::Added {
            author: agent(),
            spec: spec(),
        };

        assert_eq!(
            edit.to_string(),
            "agent claude-code@builder added 'web', running `npm run dev` in frontend"
        );
        assert!(!edit.to_string().contains("s3cret"));
        assert_eq!(edit.process().map(AsRef::as_ref), Some("web"));
    }

    /// A change names every field it edited, and the new command and
    /// working dir when it set them.
    #[test]
    fn a_change_names_its_fields_and_the_new_command_and_working_dir() {
        let edit = StackEdit::Changed {
            author: agent(),
            fields: vec![
                ProcessField::Command,
                ProcessField::WorkingDir,
                ProcessField::Environment,
            ],
            spec: spec().with_working_dir(None),
        };

        assert_eq!(
            edit.to_string(),
            "agent claude-code@builder changed command, working-dir, environment of 'web'; \
             command now `npm run dev`; working dir now the project root"
        );
        assert!(!edit.to_string().contains("s3cret"));

        let restart = StackEdit::Changed {
            author: agent(),
            fields: vec![ProcessField::Restart],
            spec: spec(),
        };
        assert_eq!(
            restart.to_string(),
            "agent claude-code@builder changed restart of 'web'"
        );
    }

    #[test]
    fn a_removal_names_the_process() {
        let edit = StackEdit::Removed {
            author: agent(),
            name: ProcessName::try_new("web").unwrap(),
        };

        assert_eq!(edit.to_string(), "agent claude-code@builder removed 'web'");
    }
}
