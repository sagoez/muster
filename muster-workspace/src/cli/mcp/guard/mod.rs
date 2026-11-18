//! `muster mcp guard`: the plugin's PreToolUse and PostToolUse hook, which
//! holds edits and tree-changing git until the agent has announced its work
//! with an in-progress todo and the checkout's lock, and reminds it to link
//! what it published.

pub(crate) mod checkout;
pub(crate) mod git_command;
pub(crate) mod guard_facts;
pub(crate) mod guarded_call;
pub(crate) mod hook_answer;
pub(crate) mod hook_event;
pub(crate) mod hook_input;
pub(crate) mod hook_reply;
pub(crate) mod pr_command;
pub(crate) mod shell_call;
pub(crate) mod verdict;
