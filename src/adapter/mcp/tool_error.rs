use thiserror::Error;

use crate::{
    adapter::client::{ClientError, FailureKind},
    domain::{
        config::ConfigError,
        value::{ProcessName, ValueParseError},
    },
};

/// Why a tool could not do what it was asked: the agent's mistake, the
/// store's refusal or a failure reaching the service.
#[derive(Debug, Error)]
pub enum ToolError {
    /// Talking to the service failed.
    #[error(transparent)]
    Client(#[from] ClientError),
    /// Stored state refused or could not be used.
    #[error(transparent)]
    Config(#[from] ConfigError),
    /// A process name was blank or held a control character.
    #[error("'{0}' is not a valid process name")]
    InvalidName(String),
    /// A command was blank.
    #[error("the command is empty")]
    EmptyCommand,
    /// A field's value was not in the form it accepts.
    #[error("invalid {field}: {reason}")]
    InvalidField {
        /// The field.
        field: &'static str,
        /// What it accepts and what it got.
        reason: ValueParseError,
    },
    /// A `process_set` call changed nothing.
    #[error("process_set changed nothing; pass at least one field besides name")]
    NothingToChange,
    /// A note key was blank.
    #[error("'{0}' is not a valid note key")]
    InvalidNoteKey(String),
    /// No note is stored under the key.
    #[error("no note '{0}'")]
    UnknownNote(String),
    /// A todo title was blank.
    #[error("'{0}' is not a valid todo title")]
    InvalidTodoTitle(String),
    /// A todo id was blank.
    #[error("'{0}' is not a valid todo id")]
    InvalidTodoId(String),
    /// No todo has the id.
    #[error("no todo '{0}'")]
    UnknownTodo(String),
    /// A key-value key was blank.
    #[error("'{0}' is not a valid key")]
    InvalidKvKey(String),
    /// No value is stored under the key.
    #[error("no value '{0}'")]
    UnknownValue(String),
}

impl ToolError {
    /// The kind of failure, which decides whether it is the agent's to fix.
    pub fn kind(&self) -> FailureKind {
        match self {
            Self::Client(error) => error.kind(),
            Self::Config(error) => FailureKind::from(error),
            Self::NothingToChange => FailureKind::Usage,
            Self::InvalidName(_)
            | Self::EmptyCommand
            | Self::InvalidField { .. }
            | Self::InvalidNoteKey(_)
            | Self::UnknownNote(_)
            | Self::InvalidTodoTitle(_)
            | Self::InvalidTodoId(_)
            | Self::UnknownTodo(_)
            | Self::InvalidKvKey(_)
            | Self::UnknownValue(_) => FailureKind::InvalidInput,
        }
    }

    /// The process names the agent could have meant.
    pub fn candidates(&self) -> &[ProcessName] {
        match self {
            Self::Client(error) => error.candidates(),
            Self::Config(ConfigError::UnknownProcess { valid, .. }) => valid,
            Self::Config(_)
            | Self::InvalidName(_)
            | Self::EmptyCommand
            | Self::InvalidField { .. }
            | Self::NothingToChange
            | Self::InvalidNoteKey(_)
            | Self::UnknownNote(_)
            | Self::InvalidTodoTitle(_)
            | Self::InvalidTodoId(_)
            | Self::UnknownTodo(_)
            | Self::InvalidKvKey(_)
            | Self::UnknownValue(_) => &[],
        }
    }
}
