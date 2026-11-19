use std::{path::Path, str::FromStr, time::Duration};

use getset::CopyGetters;
use muster_domain::coordination::AgentId;
use muster_os::clock::epoch_now;
use typed_builder::TypedBuilder;

use crate::cli::{
    command_line::cli_ports::CliPorts,
    mcp::guard::{
        guarded_call::GuardedCall,
        hook::{hook_input::HookInput, hook_reply::denial, hook_tool::HookTool},
        refusal::{memory_refusal, refusal},
    },
};

/// How long `muster mcp guard` may take before it ends printing nothing,
/// which lets the call run: well inside the plugin hook's own limit.
pub const GUARD_DEADLINE: Duration = Duration::from_secs(3);
/// The most bytes of hook input read: past any tool call an agent makes
/// but a huge write, which then reads as malformed and runs unguarded.
pub const GUARD_INPUT_MAX_BYTES: u64 = 16 * 1024 * 1024;

/// What `muster mcp guard` answers an agent's PreToolUse hook: a denial as
/// hook JSON, or none to let the call run. It is built once per hook run
/// from the environment.
#[derive(CopyGetters, TypedBuilder)]
#[getset(get_copy = "pub")]
pub struct HookAnswer<'a> {
    /// What reaches the database.
    ports: &'a dyn CliPorts,
    /// The agent calling, as the hook names it.
    me: &'a AgentId,
    /// The folder `MUSTER_PROJECT` names.
    env_project: Option<&'a Path>,
    /// The user's home folder, for `~` in a command.
    home: Option<&'a Path>,
    /// Claude Code's config folder, holding its memory files.
    claude_dir: Option<&'a Path>,
}

impl HookAnswer<'_> {
    /// The answer to the PreToolUse input `raw`. A fact about the work
    /// bound for Claude Code's memory is sent to the notes of the project
    /// [`GuardedCall::shared_fact_project`] names; otherwise the call needs
    /// the todos and locks [`GuardedCall::scopes`] names. None on input that does not read, for a
    /// call touching nothing guarded, without a current database, or on any
    /// other failure.
    pub fn answer(&self, raw: &str) -> Option<String> {
        let input: HookInput = serde_json::from_str(raw).ok()?;
        let tool = HookTool::from_str(input.tool_name()).ok()?;
        let touched = tool.touched(input.tool_input(), input.cwd(), self.home());
        if touched.is_empty() {
            return None;
        }
        let state = self.ports().existing_state().ok()?.current()?;
        let call = GuardedCall::builder()
            .store(state.as_ref())
            .tool(tool)
            .input(&input)
            .touched(touched)
            .env_project(self.env_project())
            .build();
        if let Some(project) = self
            .claude_dir()
            .and_then(|claude_dir| call.shared_fact_project(claude_dir))
        {
            return Some(denial(&memory_refusal(&project)));
        }
        let scopes = call.scopes()?;
        refusal(
            self.me(),
            scopes.todo_scopes(),
            scopes.checkouts(),
            epoch_now(),
        )
        .map(|reason| denial(&reason))
    }
}
