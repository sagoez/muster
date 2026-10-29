use std::path::PathBuf;

use thiserror::Error;

use crate::{
    adapter::{
        client::{ClientError, FailureKind, ResolveError},
        holder::PatternError,
    },
    domain::{
        config::{ConfigError, McpStackEdits, SettingKey},
        value::ValueParseError,
    },
};

/// Most candidate names a refusal lists, so a large project cannot flood the
/// caller.
pub const MAX_CANDIDATES: usize = 20;

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
    /// No registered project contains the folder a call resolved from.
    #[error(
        "no muster project contains {}; pass project, or register this folder with process_add",
        dir.display()
    )]
    NoProjectHere {
        /// The folder.
        dir: PathBuf,
    },
    /// No registered project has the name given as `project`.
    #[error("no muster project is named '{name}'")]
    UnknownProject {
        /// The name as typed.
        name: String,
        /// The registered projects' names.
        valid: Vec<String>,
    },
    /// A process name was blank or held a control character.
    #[error("'{0}' is not a valid process name")]
    InvalidName(String),
    /// A command was blank.
    #[error("the command is empty")]
    EmptyCommand,
    /// A field's value was not in the form it accepts: a regex, a
    /// condition, a cursor, a duration or any other argument.
    #[error("invalid {field}: {reason}")]
    InvalidField {
        /// The field.
        field: &'static str,
        /// What it accepts and what it got.
        reason: ValueParseError,
    },
    /// A log pattern was blank, too long, or a regex that does not compile.
    #[error("invalid {field}: {source}")]
    InvalidPattern {
        /// The field.
        field: &'static str,
        /// Why the pattern is refused.
        source: PatternError,
    },
    /// A call that edits fields was given none to change.
    #[error("{0} changed nothing; pass at least one field to change")]
    NothingToChange(&'static str),
    /// A note key was blank.
    #[error("'{0}' is not a valid note key")]
    InvalidNoteKey(String),
    /// No note is stored under the key.
    #[error("no note '{key}'")]
    UnknownNote {
        /// The key as typed.
        key: String,
        /// The keys the project has.
        valid: Vec<String>,
    },
    /// A todo title was blank.
    #[error("'{0}' is not a valid todo title")]
    InvalidTodoTitle(String),
    /// A todo id was blank.
    #[error("'{0}' is not a valid todo id")]
    InvalidTodoId(String),
    /// No todo has the id.
    #[error("no todo '{id}'")]
    UnknownTodo {
        /// The id as typed.
        id: String,
        /// The ids the project has.
        valid: Vec<String>,
    },
    /// Another agent holds the todo under a live lease.
    #[error(
        "todo '{id}' is held by {owner} for {expires_in_s} more seconds; pass force=true to change it anyway"
    )]
    HeldByOther {
        /// The todo.
        id: String,
        /// Who holds it.
        owner: String,
        /// Until the lease lapses.
        expires_in_s: u64,
    },
    /// A key-value key was blank.
    #[error("'{0}' is not a valid key")]
    InvalidKvKey(String),
    /// No value is stored under the key.
    #[error("no value '{key}'")]
    UnknownValue {
        /// The key as typed.
        key: String,
        /// The keys the project has.
        valid: Vec<String>,
    },
    /// The `mcp-stack-edits` setting keeps agents from editing the stack.
    #[error(
        "{0} is refused: the {key} setting is {value}, so agents cannot add, change or remove \
         processes; ask the user to run muster config set {key} {allow} if they want to allow it",
        key = SettingKey::McpStackEdits,
        value = McpStackEdits::ReadOnly,
        allow = McpStackEdits::Allow
    )]
    StackEditsReadOnly(&'static str),
    /// An agent asked to change a setting only the user may change.
    #[error(
        "the {0} setting cannot be changed over MCP, only the default-* settings can; ask the \
         user to change it with muster config set"
    )]
    SettingNotForAgents(SettingKey),
    /// A working directory outside the project root.
    #[error(
        "invalid working_dir: {} is outside the project root {}; an agent's process runs \
         inside its project",
        dir.display(),
        root.display()
    )]
    WorkingDirOutsideProject {
        /// The folder asked for, resolved.
        dir: PathBuf,
        /// The project's root.
        root: PathBuf,
    },
}

impl From<ResolveError> for ToolError {
    fn from(error: ResolveError) -> Self {
        match error {
            ResolveError::UnknownProject { name, valid } => Self::UnknownProject {
                name,
                valid: valid.iter().map(ToString::to_string).collect(),
            },
            ResolveError::NotAProject { dir } => Self::NoProjectHere { dir },
            ResolveError::Config(error) => Self::Config(error),
        }
    }
}

impl ToolError {
    /// The kind of failure, which decides whether it is the agent's to fix.
    pub fn kind(&self) -> FailureKind {
        match self {
            Self::Client(error) => error.kind(),
            Self::Config(error) => FailureKind::from(error),
            Self::NoProjectHere { .. } | Self::UnknownProject { .. } => FailureKind::UnknownProject,
            Self::UnknownNote { .. } => FailureKind::UnknownNote,
            Self::UnknownTodo { .. } => FailureKind::UnknownTodo,
            Self::UnknownValue { .. } => FailureKind::UnknownKey,
            Self::NothingToChange(_) => FailureKind::Usage,
            Self::InvalidName(_)
            | Self::EmptyCommand
            | Self::InvalidField { .. }
            | Self::InvalidPattern { .. }
            | Self::InvalidNoteKey(_)
            | Self::InvalidTodoTitle(_)
            | Self::InvalidTodoId(_)
            | Self::HeldByOther { .. }
            | Self::InvalidKvKey(_)
            | Self::WorkingDirOutsideProject { .. } => FailureKind::InvalidInput,
            Self::StackEditsReadOnly(_) | Self::SettingNotForAgents(_) => FailureKind::NotAllowed,
        }
    }

    /// The names the agent could have meant, at most [`MAX_CANDIDATES`].
    pub fn candidates(&self) -> Vec<String> {
        let names: Vec<String> = match self {
            Self::Client(error) => error.candidates().iter().map(ToString::to_string).collect(),
            Self::Config(ConfigError::UnknownProcess { valid, .. }) => {
                valid.iter().map(ToString::to_string).collect()
            },
            Self::UnknownProject { valid, .. }
            | Self::UnknownNote { valid, .. }
            | Self::UnknownTodo { valid, .. }
            | Self::UnknownValue { valid, .. } => valid.clone(),
            Self::Config(_)
            | Self::NoProjectHere { .. }
            | Self::InvalidName(_)
            | Self::EmptyCommand
            | Self::InvalidField { .. }
            | Self::InvalidPattern { .. }
            | Self::NothingToChange(_)
            | Self::InvalidNoteKey(_)
            | Self::InvalidTodoTitle(_)
            | Self::InvalidTodoId(_)
            | Self::HeldByOther { .. }
            | Self::InvalidKvKey(_)
            | Self::StackEditsReadOnly(_)
            | Self::SettingNotForAgents(_)
            | Self::WorkingDirOutsideProject { .. } => Vec::new(),
        };
        names.into_iter().take(MAX_CANDIDATES).collect()
    }
}
