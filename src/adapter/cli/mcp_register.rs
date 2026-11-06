//! `muster mcp add` and `muster mcp remove`: muster replaced, by `exec`,
//! with the agent's own CLI registering or unregistering its MCP server, so
//! the agent owns the terminal and muster's exit is the agent's own.

use std::{
    ffi::OsString, io::ErrorKind, iter, os::unix::process::CommandExt, path::PathBuf,
    process::Command,
};

use serde_json::json;
use strum::{EnumMessage, EnumProperty};

use super::{
    emit::emit_line,
    error::CliError,
    mcp_agent::{McpAgent, PROGRAM},
    mcp_command::McpCommand,
    mcp_scope::McpScope,
    report::print_note,
};
use crate::{
    adapter::client::{runnable, this_binary},
    constants::{APP_NAME, MCP_CONNECTION_KEY, MCP_SUBCOMMAND},
};

/// The scope an add registers in when none is given: every project, as the
/// server finds each call's project itself.
const ADD_SCOPE: McpScope = McpScope::User;
/// The agents' subcommand for MCP servers.
const AGENT_MCP: &str = "mcp";
/// The agents' verb that registers a server.
const ADD_VERB: &str = "add";
/// The agents' verb that unregisters one.
const REMOVE_VERB: &str = "remove";
/// Claude Code's flag naming the scope.
const SCOPE_FLAG: &str = "--scope";
/// Ends the agent's own options; the server's command line follows.
const END_OF_OPTIONS: &str = "--";
/// Between the words of a command shlex cannot quote.
const WORD_SEPARATOR: &str = " ";
/// Why `--project` is refused: the agent works where it is run.
const PROJECT_REFUSED: &str = "`muster mcp add` and `remove` run the agent in the current folder, so they take no --project; run them in the project's folder";
/// What to do with the printed entry in Zed, which has no command for it.
const ZED_ADD_NOTE: &str = "Zed has no command for this: paste the entry above as a key inside the \"context_servers\" object of Zed's settings.json; see https://zed.dev/docs/ai/mcp";
/// What to delete in Zed to unregister muster.
const ZED_REMOVE_NOTE: &str = "Zed has no command for this: delete the \"muster\" key from the \"context_servers\" object of Zed's settings.json; see https://zed.dev/docs/ai/mcp";

/// Replaces muster with the agent's CLI making `command`, or for Zed prints
/// what to change by hand. Returns only for Zed or on a failure.
///
/// # Errors
/// Returns [`CliError::Usage`] when `project_given`, as the agent works in
/// the current folder, [`CliError::UserScopeOnly`] for a scope other than
/// `user` with an agent other than Claude Code, [`CliError::BinaryGone`]
/// when this binary was replaced or removed, and [`CliError::AgentMissing`]
/// or [`CliError::AgentNotRunnable`] when the agent cannot be run.
pub fn register(command: &McpCommand, project_given: bool) -> Result<(), CliError> {
    if project_given {
        return Err(CliError::Usage(PROJECT_REFUSED.to_owned()));
    }
    let (adding, args) = match command {
        McpCommand::Add(args) => (true, args),
        McpCommand::Remove(args) => (false, args),
    };
    let agent = args.agent();
    if agent != McpAgent::Claude && args.scope().is_some_and(|scope| scope != McpScope::User) {
        return Err(CliError::UserScopeOnly(
            agent.get_message().unwrap_or_default(),
        ));
    }
    let Some(program) = agent.get_str(PROGRAM) else {
        if !adding {
            print_note(APP_NAME, ZED_REMOVE_NOTE);
            return Ok(());
        }
        let entry = json!({
            "command": own_binary()?.display().to_string(),
            "args": [MCP_SUBCOMMAND],
        });
        let entry = serde_json::to_string_pretty(&entry).unwrap_or_default();
        emit_line(&format!("\"{MCP_CONNECTION_KEY}\": {entry}"));
        print_note(APP_NAME, ZED_ADD_NOTE);
        return Ok(());
    };
    // Without a scope Claude Code removes the server from whichever scope
    // holds it; an add needs one. A project's shared file names the bare
    // `muster` for each person's own `PATH` to find.
    let (scope, binary) = match (adding, args.scope()) {
        (false, scope) => (scope, None),
        (true, Some(McpScope::Project)) => (Some(McpScope::Project), Some(APP_NAME.into())),
        (true, scope) => (Some(scope.unwrap_or(ADD_SCOPE)), Some(own_binary()?)),
    };
    let words = agent_words(agent, scope, binary);
    let source = Command::new(program).args(&words).exec();
    Err(if source.kind() == ErrorKind::NotFound {
        let command = shell_words(program, &words);
        CliError::AgentMissing {
            program,
            source,
            command,
        }
    } else {
        CliError::AgentNotRunnable { program, source }
    })
}

/// This binary's own path, as a config names it.
///
/// # Errors
/// Returns [`CliError::BinaryGone`] when the binary was replaced or removed
/// since it started and nothing runnable is left at its path.
fn own_binary() -> Result<PathBuf, CliError> {
    let binary = this_binary();
    runnable(&binary).ok_or(CliError::BinaryGone(binary))
}

/// The arguments of `agent`'s command registering `binary` in `scope`, or
/// without a binary unregistering it. Codex's CLI has no scopes.
fn agent_words(agent: McpAgent, scope: Option<McpScope>, binary: Option<PathBuf>) -> Vec<OsString> {
    let verb = if binary.is_some() {
        ADD_VERB
    } else {
        REMOVE_VERB
    };
    let mut words: Vec<OsString> = vec![AGENT_MCP.into(), verb.into()];
    if agent == McpAgent::Claude
        && let Some(scope) = scope
    {
        words.extend([SCOPE_FLAG.into(), scope.to_string().into()]);
    }
    words.push(MCP_CONNECTION_KEY.into());
    if let Some(binary) = binary {
        words.extend([END_OF_OPTIONS.into(), binary.into(), MCP_SUBCOMMAND.into()]);
    }
    words
}

/// `program` and `words` as a person would type them.
fn shell_words(program: &str, words: &[OsString]) -> String {
    let words: Vec<String> = iter::once(program.to_owned())
        .chain(words.iter().map(|word| word.to_string_lossy().into_owned()))
        .collect();
    shlex::try_join(words.iter().map(String::as_str)).unwrap_or_else(|_| words.join(WORD_SEPARATOR))
}
