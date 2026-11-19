//! `muster mcp guard`: the plugin's PreToolUse hook, which holds edits and
//! tree-changing git until the agent has announced its work with an
//! in-progress todo and the checkout's lock, and sends a fact about the
//! work bound for Claude Code's private memory to a muster note, where
//! every agent reads it. It only reads.

pub(crate) mod call_scopes;
pub(crate) mod checkout_scope;
pub(crate) mod guarded_call;
pub(crate) mod hook;
pub(crate) mod hook_answer;
pub(crate) mod memory_file;
pub(crate) mod refusal;
pub(crate) mod shell;
pub(crate) mod todo_scope;
pub(crate) mod tool_call;
