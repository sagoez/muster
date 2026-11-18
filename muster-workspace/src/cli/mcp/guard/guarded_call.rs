use std::path::Path;

use serde_json::Value;

use crate::cli::mcp::guard::shell_call::ShellCall;

/// The tools that edit files: Claude Code's, and Codex's `apply_patch`.
pub const EDIT_TOOLS: [&str; 5] = ["Edit", "Write", "MultiEdit", "NotebookEdit", "apply_patch"];
/// The tool that runs a shell command, in both agents.
pub const SHELL_TOOL: &str = "Bash";
/// The key of a shell tool's command in its input.
const COMMAND_KEY: &str = "command";
/// Joins the words of a command given as a list.
const WORD_SEPARATOR: &str = " ";

/// A tool call the guard holds to the agent's announced work.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GuardedCall {
    /// A file edit, which needs an in-progress todo.
    Edit,
    /// A git or gh command changing or publishing a checkout, which needs an
    /// in-progress todo and the checkout's lock.
    Shell(ShellCall),
}

impl GuardedCall {
    /// The guarded call `tool` makes with `input` in `cwd`, or none when it
    /// is not one: another tool, a shell command touching no checkout, or a
    /// command that does not read.
    pub fn of(tool: &str, input: &Value, cwd: &Path) -> Option<Self> {
        if EDIT_TOOLS.contains(&tool) {
            return Some(Self::Edit);
        }
        if tool != SHELL_TOOL {
            return None;
        }
        let command = match input.get(COMMAND_KEY)? {
            Value::String(command) => command.clone(),
            Value::Array(words) => words
                .iter()
                .map(|word| word.as_str().and_then(|word| shlex::try_quote(word).ok()))
                .collect::<Option<Vec<_>>>()?
                .join(WORD_SEPARATOR),
            _ => return None,
        };
        ShellCall::parse(&command, cwd).map(Self::Shell)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// The folder every test call runs in.
    const CWD: &str = "/work/shop";

    /// The call `tool` makes with `input`.
    fn call(tool: &str, input: &Value) -> Option<GuardedCall> {
        GuardedCall::of(tool, input, Path::new(CWD))
    }

    /// Every edit tool is guarded whatever it edits, and reading tools are
    /// not.
    #[test]
    fn edit_tools_are_guarded_and_reading_ones_are_not() {
        for tool in EDIT_TOOLS {
            assert_eq!(call(tool, &json!({})), Some(GuardedCall::Edit), "{tool}");
        }
        assert_eq!(call("Read", &json!({"file_path": "/a"})), None);
    }

    /// A shell command is guarded only when it changes or publishes a
    /// checkout, given as one string or as a list of words.
    #[test]
    fn a_shell_command_is_guarded_by_what_it_runs() {
        let commit = call(SHELL_TOOL, &json!({"command": "git commit -m x"}));
        assert!(matches!(commit, Some(GuardedCall::Shell(_))), "{commit:?}");
        let listed = call(SHELL_TOOL, &json!({"command": ["git", "push"]}));
        assert!(
            matches!(&listed, Some(GuardedCall::Shell(shell)) if shell.publishes()),
            "{listed:?}"
        );
        assert_eq!(call(SHELL_TOOL, &json!({"command": "git status"})), None);
        assert_eq!(call(SHELL_TOOL, &json!({"command": 3})), None);
        assert_eq!(call(SHELL_TOOL, &json!({})), None);
    }
}
