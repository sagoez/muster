//! Every tool muster's server offers, by the name agents call it: the one
//! source each tool name is read from.

#[cfg(any(test, feature = "testing"))]
use std::{collections::HashSet, str::FromStr};

#[cfg(any(test, feature = "testing"))]
use strum::IntoEnumIterator;
use strum::{AsRefStr, Display, EnumIter, EnumProperty, EnumString, IntoStaticStr};

/// The property marking a tool an agent holds from the start of its session.
const LOADED_UP_FRONT: &str = "loaded_up_front";
/// Joins a tool's noun to its verb, as in `note_read`.
#[cfg(any(test, feature = "testing"))]
const NOUN_BREAK: char = '_';

/// A tool of muster's server, spelled as agents call it. MCP tool names are
/// snake_case, not kebab-case, so each variant carries its spelling, as
/// rmcp names a tool after its function.
#[derive(
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    Hash,
    Display,
    AsRefStr,
    EnumString,
    EnumIter,
    EnumProperty,
    IntoStaticStr,
)]
pub enum ToolName {
    /// Reads a machine setting.
    #[strum(serialize = "config_get")]
    ConfigGet,
    /// Changes a machine setting.
    #[strum(serialize = "config_set")]
    ConfigSet,
    /// Deletes a key-value entry.
    #[strum(serialize = "kv_delete")]
    KvDelete,
    /// Reads or lists key-value entries.
    #[strum(serialize = "kv_read")]
    KvRead,
    /// Writes a key-value entry.
    #[strum(serialize = "kv_write")]
    KvWrite,
    /// Takes a named lock.
    #[strum(serialize = "lock_acquire")]
    LockAcquire,
    /// Frees a named lock.
    #[strum(serialize = "lock_release")]
    LockRelease,
    /// Deletes a note.
    #[strum(serialize = "note_delete")]
    NoteDelete,
    /// Reads or lists the notes, what other agents left behind.
    #[strum(serialize = "note_read", props(loaded_up_front = true))]
    NoteRead,
    /// Writes a note, leaving what was learned for other agents.
    #[strum(serialize = "note_write", props(loaded_up_front = true))]
    NoteWrite,
    /// Adds a process to a stack.
    #[strum(serialize = "process_add")]
    ProcessAdd,
    /// Reads a process's log.
    #[strum(serialize = "process_logs", props(loaded_up_front = true))]
    ProcessLogs,
    /// Opens a process in a herdr pane.
    #[strum(serialize = "process_open")]
    ProcessOpen,
    /// Removes a process from a stack.
    #[strum(serialize = "process_remove")]
    ProcessRemove,
    /// Restarts a process.
    #[strum(serialize = "process_restart")]
    ProcessRestart,
    /// Reads a process's screen.
    #[strum(serialize = "process_screen")]
    ProcessScreen,
    /// Sends input to a process.
    #[strum(serialize = "process_send")]
    ProcessSend,
    /// Changes a process's definition.
    #[strum(serialize = "process_set")]
    ProcessSet,
    /// Starts a process.
    #[strum(serialize = "process_start")]
    ProcessStart,
    /// Stops a process.
    #[strum(serialize = "process_stop")]
    ProcessStop,
    /// Waits for a process's line, port, answer or state.
    #[strum(serialize = "process_wait")]
    ProcessWait,
    /// Lists every registered project.
    #[strum(serialize = "project_list", props(loaded_up_front = true))]
    ProjectList,
    /// Registers a folder as a project.
    #[strum(serialize = "project_register")]
    ProjectRegister,
    /// Long-polls a stack's lifecycle events.
    #[strum(serialize = "stack_events")]
    StackEvents,
    /// Shows a whole stack, what runs and how.
    #[strum(serialize = "stack_status", props(loaded_up_front = true))]
    StackStatus,
    /// Starts a stack's autostart processes.
    #[strum(serialize = "stack_up")]
    StackUp,
    /// Adds a todo.
    #[strum(serialize = "todo_add")]
    TodoAdd,
    /// Claims a todo.
    #[strum(serialize = "todo_claim")]
    TodoClaim,
    /// Deletes a todo.
    #[strum(serialize = "todo_delete")]
    TodoDelete,
    /// Lists the todos, what is left to do.
    #[strum(serialize = "todo_list", props(loaded_up_front = true))]
    TodoList,
    /// Changes a todo.
    #[strum(serialize = "todo_update")]
    TodoUpdate,
}

impl ToolName {
    /// Whether an agent holds this tool from the start of its session,
    /// rather than only once a tool search finds it.
    pub fn loaded_up_front(self) -> bool {
        self.get_bool(LOADED_UP_FRONT).unwrap_or_default()
    }
}

#[cfg(any(test, feature = "testing"))]
impl ToolName {
    /// The words of `text` written as a tool name, `noun_verb` under the
    /// noun of some tool, that name no tool: what a renamed tool would leave
    /// behind in the prose agents read, which a test keeps empty.
    pub fn strays_in(text: &str) -> Vec<&str> {
        let nouns: HashSet<&str> = Self::iter()
            .filter_map(|tool| <&'static str>::from(tool).split_once(NOUN_BREAK))
            .map(|(noun, _)| noun)
            .collect();
        text.split(|character: char| !(character.is_ascii_lowercase() || character == NOUN_BREAK))
            .filter(|word| {
                word.split_once(NOUN_BREAK)
                    .is_some_and(|(noun, verb)| nouns.contains(noun) && !verb.is_empty())
            })
            .filter(|word| Self::from_str(word).is_err())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A word written as a tool name under a tool's noun is a stray unless
    /// it names a tool; other words, and other nouns, are not read.
    #[test]
    fn only_dead_tool_names_are_strays() {
        assert_eq!(
            ToolName::strays_in("call note_read, then note_sweep; errors_only=true, process_"),
            vec!["note_sweep"]
        );
        assert!(ToolName::strays_in("todo_list and lock_release (stack_status).").is_empty());
    }
}
