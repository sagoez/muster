//! `muster mcp add` and `muster mcp remove`: the agent's own CLI asked to
//! register or unregister muster's MCP server, speaking for itself on the
//! person's terminal.

use std::{
    ffi::OsString,
    iter,
    path::{Path, PathBuf},
    process::Command,
};

use super::{
    cli_exit::CliExit, emit::emit_line, error::CliError, mcp_agent::McpAgent,
    mcp_command::McpCommand, mcp_scope::McpScope, report::print_note,
};
use crate::{adapter::client::binary_as_typed, constants::APP_NAME};

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
/// The name muster's server is registered under.
const SERVER_NAME: &str = APP_NAME;
/// The muster subcommand that serves MCP.
const SERVE_ARG: &str = "mcp";
/// Between the words of a command shlex cannot quote.
const WORD_SEPARATOR: &str = " ";
/// The key Zed keeps its MCP servers under.
const ZED_SERVERS: &str = "context_servers";
/// Zed's documentation for adding an MCP server.
const ZED_MCP_DOCS: &str = "https://zed.dev/docs/ai/mcp";

/// Runs `command` through the agent it names and answers the exit code to
/// end with: the agent's own, or success for Zed, which is only told what
/// to change by hand.
///
/// # Errors
/// Returns [`CliError::UserScopeOnly`] for a scope other than `user` with
/// an agent other than Claude Code, or [`CliError::AgentNotRunnable`] when
/// the agent's binary cannot be run, naming the command to run once it can.
pub fn register(command: &McpCommand) -> Result<i32, CliError> {
    let (adding, args) = match command {
        McpCommand::Add(args) => (true, args),
        McpCommand::Remove(args) => (false, args),
    };
    let agent = args.agent();
    let scope = args.scope().unwrap_or_default();
    if agent != McpAgent::Claude && scope != McpScope::User {
        return Err(CliError::UserScopeOnly(agent.title()));
    }
    let Some(program) = agent.program() else {
        zed_by_hand(adding);
        return Ok(CliExit::Success.code());
    };
    let words = agent_words(agent, scope, adding, &server_binary(scope));
    match Command::new(program).args(&words).status() {
        Ok(status) => Ok(status.code().unwrap_or(CliExit::ActionFailed.code())),
        Err(source) => Err(CliError::AgentNotRunnable {
            program,
            command: shell_words(program, &words),
            source,
        }),
    }
}

/// The muster binary an entry of `scope` runs: the bare `muster` in a
/// project's shared file, for each person's own `PATH` to find.
fn server_binary(scope: McpScope) -> PathBuf {
    if scope == McpScope::Project {
        PathBuf::from(APP_NAME)
    } else {
        binary_as_typed()
    }
}

/// The arguments of `agent`'s command registering `binary` in `scope`, or
/// with `adding` false unregistering it. Codex's CLI has no scopes.
fn agent_words(agent: McpAgent, scope: McpScope, adding: bool, binary: &Path) -> Vec<OsString> {
    let verb = if adding { ADD_VERB } else { REMOVE_VERB };
    let mut words: Vec<OsString> = vec![AGENT_MCP.into(), verb.into()];
    if agent == McpAgent::Claude {
        words.extend([SCOPE_FLAG.into(), scope.to_string().into()]);
    }
    words.push(SERVER_NAME.into());
    if adding {
        words.extend([END_OF_OPTIONS.into(), binary.into(), SERVE_ARG.into()]);
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

/// Tells the person what to change in Zed's settings by hand: with
/// `adding`, the entry to add, printed on stdout so it can be piped.
fn zed_by_hand(adding: bool) {
    if !adding {
        print_note(
            APP_NAME,
            &format!(
                "delete \"{SERVER_NAME}\" from {ZED_SERVERS} in Zed's settings; see {ZED_MCP_DOCS}"
            ),
        );
        return;
    }
    let entry = serde_json::json!({
        ZED_SERVERS: {
            SERVER_NAME: {
                "command": binary_as_typed().display().to_string(),
                "args": [SERVE_ARG],
                "env": {},
            },
        },
    });
    emit_line(&serde_json::to_string_pretty(&entry).unwrap_or_default());
    print_note(
        APP_NAME,
        &format!("add this to {ZED_SERVERS} in Zed's settings; see {ZED_MCP_DOCS}"),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Claude Code is given the scope, Codex none; only an add carries the
    /// server's command line, after `--`.
    #[test]
    fn each_agent_gets_its_own_words() {
        let binary = Path::new("/opt/muster");
        let claude = agent_words(McpAgent::Claude, McpScope::Local, true, binary);
        assert_eq!(
            shell_words("claude", &claude),
            "claude mcp add --scope local muster -- /opt/muster mcp"
        );
        let codex = agent_words(McpAgent::Codex, McpScope::User, false, binary);
        assert_eq!(shell_words("codex", &codex), "codex mcp remove muster");
        assert_eq!(server_binary(McpScope::Project), PathBuf::from(APP_NAME));
    }
}
