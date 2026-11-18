use std::path::PathBuf;

use getset::Getters;
use serde::Deserialize;
use serde_json::Value;
use typed_builder::TypedBuilder;

/// The part of a PreToolUse hook's JSON input the guard reads, as Claude
/// Code and Codex both send it. Other fields are ignored, as either agent
/// may add more; the plugin wires the guard to PreToolUse alone.
#[derive(Clone, Debug, PartialEq, Deserialize, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct HookInput {
    /// The tool called, such as `Bash` or `Edit`.
    tool_name: String,
    /// The tool's arguments: a file edit's `file_path` or `notebook_path`,
    /// and the `command` of `Bash` and of Codex's `apply_patch`.
    tool_input: Value,
    /// The folder the call runs in: the session's, or Codex's `workdir`
    /// for the call when one is given.
    cwd: PathBuf,
}
