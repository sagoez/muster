//! Every tool muster's server offers, by the name agents call it: the one
//! source each tool name is read from.

#[cfg(any(test, feature = "testing"))]
use std::{
    collections::HashSet,
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
};

#[cfg(any(test, feature = "testing"))]
use strum::IntoEnumIterator;
use strum::{AsRefStr, Display, EnumIter, EnumProperty, EnumString, IntoStaticStr};

/// The property marking a tool an agent holds from the start of its session.
const LOADED_UP_FRONT: &str = "loaded_up_front";
/// Joins a tool's noun to its verb, as in `note_read`.
#[cfg(any(test, feature = "testing"))]
const NOUN_BREAK: char = '_';
/// The extension of a Rust file.
#[cfg(any(test, feature = "testing"))]
const RUST_EXTENSION: &str = "rs";
/// Opens and closes a string literal.
#[cfg(any(test, feature = "testing"))]
const QUOTE: char = '"';
/// Escapes the character after it in a string literal.
#[cfg(any(test, feature = "testing"))]
const ESCAPE: char = '\\';
/// Opens a character literal, so a quote after it opens no string.
#[cfg(any(test, feature = "testing"))]
const CHAR_QUOTE: char = '\'';
/// The folder of a crate's source.
#[cfg(test)]
const SOURCE_DIR: &str = "src";

/// A tool of muster's server, spelled as agents call it. MCP tool names are
/// snake_case, not kebab-case, so each variant carries its spelling, as
/// rmcp names a tool after its function.
#[derive(
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
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
    /// noun of some tool, that are neither a tool's name nor one extended
    /// with `_`, as a snapshot is named: what renaming a tool's verb would
    /// leave behind. A word under a noun no tool has any more is not caught.
    pub fn strays_in(text: &str) -> Vec<&str> {
        let names: Vec<&'static str> = Self::iter().map(<&'static str>::from).collect();
        let nouns: HashSet<&str> = names
            .iter()
            .filter_map(|name| name.split_once(NOUN_BREAK))
            .map(|(noun, _)| noun)
            .collect();
        text.split(|character: char| !(character.is_ascii_lowercase() || character == NOUN_BREAK))
            .filter(|word| {
                word.split_once(NOUN_BREAK)
                    .is_some_and(|(noun, verb)| nouns.contains(noun) && !verb.is_empty())
            })
            .filter(|word| {
                !names.iter().any(|name| {
                    word.strip_prefix(name)
                        .is_some_and(|rest| rest.is_empty() || rest.starts_with(NOUN_BREAK))
                })
            })
            .collect()
    }

    /// Every stray, as [`Self::strays_in`] finds them, in the string
    /// literals of the Rust files below `root`, each as `path: word`. A test
    /// keeps it empty, so no text an agent reads names a tool that is gone,
    /// with no list of those texts to keep.
    pub fn strays_below(root: &Path) -> Vec<String> {
        rust_files_below(root)
            .into_iter()
            .flat_map(|path| {
                let code = fs::read_to_string(&path).unwrap_or_default();
                string_literals(&code)
                    .into_iter()
                    .flat_map(Self::strays_in)
                    .map(|word| format!("{}: {word}", path.display()))
                    .collect::<Vec<String>>()
            })
            .collect()
    }
}

/// Every Rust file below `dir`, through any folder below it.
#[cfg(any(test, feature = "testing"))]
fn rust_files_below(dir: &Path) -> Vec<PathBuf> {
    fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .flat_map(|path| {
            if path.is_dir() {
                rust_files_below(&path)
            } else if path.extension() == Some(OsStr::new(RUST_EXTENSION)) {
                vec![path]
            } else {
                Vec::new()
            }
        })
        .collect()
}

/// The contents of the string literals in Rust `code`, read plainly: an
/// escaped quote stays inside its literal, and a quote character literal
/// opens none.
#[cfg(any(test, feature = "testing"))]
fn string_literals(code: &str) -> Vec<&str> {
    let mut literals = Vec::new();
    let mut opened: Option<usize> = None;
    let mut escaped = false;
    let mut previous = None;
    for (at, character) in code.char_indices() {
        match opened {
            Some(start) if character == QUOTE && !escaped => {
                literals.push(&code[start..at]);
                opened = None;
            },
            Some(_) => escaped = character == ESCAPE && !escaped,
            None if character == QUOTE && previous != Some(CHAR_QUOTE) => {
                opened = Some(at + character.len_utf8());
                escaped = false;
            },
            None => {},
        }
        previous = Some(character);
    }
    literals
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A word written as a tool name under a tool's noun is a stray unless
    /// it names a tool or extends one with `_`; other words, and other
    /// nouns, are not read.
    #[test]
    fn only_dead_tool_names_are_strays() {
        let swept = concat!("note", "_sweep");
        let text = format!("call note_read, then {swept}; errors_only=true, process_");
        assert_eq!(ToolName::strays_in(&text), vec![swept]);
        assert!(
            ToolName::strays_in("todo_list, lock_release (stack_status), process_open_schema.")
                .is_empty()
        );
    }

    /// A string literal is read whole, an escaped quote inside it, and a
    /// quote character literal opens none.
    #[test]
    fn string_literals_are_read_whole() {
        let code = r#"let a = "one \"two\""; let q = '"'; f("three");"#;
        assert_eq!(string_literals(code), vec![r#"one \"two\""#, "three"]);
    }

    /// No string in muster's server names a tool it does not offer.
    #[test]
    fn no_text_of_the_server_names_a_tool_that_is_gone() {
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join(SOURCE_DIR);
        assert_eq!(ToolName::strays_below(&source), Vec::<String>::new());
    }
}
