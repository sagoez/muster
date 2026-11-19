//! What an agent can do about a failed call, as its refusal tells it.

use std::fmt::{self, Display, Formatter};

use strum::{EnumIter, EnumMessage};

/// What an agent does about a failure, appended to the failure's message.
/// Each variant's text is its `message`, which it displays as.
#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumIter, EnumMessage)]
pub(crate) enum AgentHint {
    /// A muster service that runs another version, which no tool can fix.
    #[strum(message = "restart the muster MCP server")]
    RestartServer,
    /// An older muster service left running.
    #[strum(
        message = "call stack_status, which replaces it while every process keeps running, \
                     then call again"
    )]
    ReplaceOlderService,
    /// A service that is shutting down.
    #[strum(message = "the muster service is shutting down; call the tool again")]
    CallAgain,
    /// A process that is not running.
    #[strum(message = "call process_start to start it")]
    StartProcess,
    /// A folder that is not a project.
    #[strum(
        message = "call project_register to register this folder as a project, which needs no \
                     process"
    )]
    RegisterProject,
    /// A name the project already has.
    #[strum(message = "choose another name")]
    ChooseAnotherName,
    /// A note key that names no note.
    #[strum(message = "note_read without key lists the notes")]
    ListNotes,
    /// A todo id that names no todo.
    #[strum(message = "todo_list lists the todos")]
    ListTodos,
    /// A key that names no value.
    #[strum(message = "kv_read without key lists the entries")]
    ListValues,
    /// A project name that names no project.
    #[strum(message = "pass a registered project's name or a folder as project")]
    PassProject,
    /// A cursor past the end of the output.
    #[strum(message = "pass a cursor from an earlier result")]
    PassCursor,
}

impl Display for AgentHint {
    /// Writes the hint's text.
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.get_message().unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use strum::IntoEnumIterator;

    use super::*;

    /// Every hint carries its text, so none displays as nothing.
    #[test]
    fn every_hint_has_its_text() {
        for hint in AgentHint::iter() {
            assert!(hint.get_message().is_some(), "{hint:?}");
        }
    }
}
