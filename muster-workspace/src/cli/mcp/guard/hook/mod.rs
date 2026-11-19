//! The hook's side of the wire: what an agent sends, which tools it names,
//! what a write leaves in a file, and the refusal printed back.

pub(crate) mod hook_input;
pub(crate) mod hook_reply;
pub(crate) mod hook_tool;
pub(crate) mod written_text;
