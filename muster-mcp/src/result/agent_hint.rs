//! What an agent can do about a failed call, as its refusal tells it.

use strum::{Display, EnumIter};

/// What an agent does about a failure, appended to the failure's message.
/// Each variant's text is its `Display`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Display, EnumIter)]
pub(crate) enum AgentHint {
    /// A muster service that runs another version, which no tool can fix.
    #[strum(to_string = "restart the muster MCP server")]
    RestartServer,
    /// An older muster service left running.
    #[strum(
        to_string = "call stack_status, which replaces it while every process keeps running, \
                     then call again"
    )]
    ReplaceOlderService,
    /// A service that is shutting down.
    #[strum(to_string = "the muster service is shutting down; call the tool again")]
    CallAgain,
    /// A process that is not running.
    #[strum(to_string = "call process_start to start it")]
    StartProcess,
    /// A folder that is not a project.
    #[strum(
        to_string = "call project_register to register this folder as a project, which needs no \
                     process"
    )]
    RegisterProject,
    /// A name the project already has.
    #[strum(to_string = "choose another name")]
    ChooseAnotherName,
    /// A note key that names no note.
    #[strum(to_string = "note_read without key lists the notes")]
    ListNotes,
    /// A todo id that names no todo.
    #[strum(to_string = "todo_list lists the todos")]
    ListTodos,
    /// A key that names no value.
    #[strum(to_string = "kv_read without key lists the entries")]
    ListValues,
    /// A project name that names no project.
    #[strum(to_string = "pass a registered project's name or a folder as project")]
    PassProject,
    /// A cursor past the end of the output.
    #[strum(to_string = "pass a cursor from an earlier result")]
    PassCursor,
}

#[cfg(test)]
mod tests {
    use strum::IntoEnumIterator;

    use super::*;
    use crate::runtime::ToolName;

    /// Every tool a hint names to an agent is a tool the server offers.
    #[test]
    fn every_hinted_tool_exists() {
        for hint in AgentHint::iter() {
            let text = hint.to_string();
            assert!(ToolName::strays_in(&text).is_empty(), "{text}");
        }
    }
}
