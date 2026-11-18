//! `muster mcp guard`: the plugin's PreToolUse hook, which holds edits and
//! tree-changing git until the agent has announced its work with an
//! in-progress todo and the checkout's lock. It only reads.

pub(crate) mod checkout_scope;
pub(crate) mod hook;
pub(crate) mod hook_answer;
pub(crate) mod refusal;
pub(crate) mod shell;
pub(crate) mod todo_scope;
