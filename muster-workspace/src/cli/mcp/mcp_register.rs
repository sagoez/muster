//! `muster mcp add` and `muster mcp remove`: the agent's own CLI registering
//! or unregistering muster's MCP server, with its output and exit passed
//! through, and muster's session-start hook put into or taken out of the
//! agent's hooks file around it.

use std::{
    env,
    ffi::{OsStr, OsString},
    io::{self, ErrorKind},
    iter,
    os::unix::process::{CommandExt, ExitStatusExt},
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Stdio, exit},
};

use muster_client::{runnable, this_binary};
use muster_domain::constants::{APP_NAME, MCP_CONNECTION_KEY, MCP_SUBCOMMAND};
use muster_os::path::home_dir;
use serde_json::json;
use signal_hook::low_level::emulate_default_handler;
use strum::{EnumMessage, EnumProperty, IntoEnumIterator};

use crate::cli::{
    failure::error::CliError,
    mcp::{
        hook_file::{HookFile, context_command},
        hook_file_error::HookFileError,
        mcp_agent::{McpAgent, PROGRAM},
        mcp_registration::McpRegistration,
        mcp_scope::McpScope,
    },
    present::emit::emit_line,
    rows::report::print_note,
};

/// The scope an add registers in when none is given: every project, as the
/// server finds each call's project itself.
const ADD_SCOPE: McpScope = McpScope::User;
/// The agents' subcommand for MCP servers.
const AGENT_MCP: &str = "mcp";
/// Codex's verb that registers a server.
const ADD_VERB: &str = "add";
/// Claude Code's verb that registers a server from its JSON entry, which
/// alone carries `alwaysLoad`.
const ADD_JSON_VERB: &str = "add-json";
/// The agents' verb that unregisters one.
const REMOVE_VERB: &str = "remove";
/// Claude Code's flag naming the scope.
const SCOPE_FLAG: &str = "--scope";
/// Ends the agent's own options; the server's command line follows.
const END_OF_OPTIONS: &str = "--";
/// Between the words of a command shlex cannot quote.
const WORD_SEPARATOR: &str = " ";
/// The keys and values of Claude Code's server entry.
const TYPE_KEY: &str = "type";
const STDIO_TYPE: &str = "stdio";
const COMMAND_KEY: &str = "command";
const ARGS_KEY: &str = "args";
/// The key that exempts a server's tools from Claude Code's tool search, so
/// a session holds them from its start.
const ALWAYS_LOAD_KEY: &str = "alwaysLoad";
/// The variable that moves Claude Code's config folder off `~/.claude`.
const CLAUDE_CONFIG_ENV: &str = "CLAUDE_CONFIG_DIR";
/// Claude Code's config folder, in the home folder and in a project.
const CLAUDE_DIR: &str = ".claude";
/// Claude Code's settings file, shared in a project or the user's own.
const CLAUDE_SETTINGS: &str = "settings.json";
/// Claude Code's settings file a project keeps out of version control.
const CLAUDE_LOCAL_SETTINGS: &str = "settings.local.json";
/// The variable that moves Codex's home folder off `~/.codex`.
const CODEX_HOME_ENV: &str = "CODEX_HOME";
/// Codex's home folder, in the home folder.
const CODEX_DIR: &str = ".codex";
/// Codex's hooks file.
const CODEX_HOOKS: &str = "hooks.json";
/// What a missing home folder leaves unfound.
const CLAUDE_FOLDER: &str = "Claude Code's ~/.claude";
const CODEX_FOLDER: &str = "Codex's ~/.codex";
/// Why `--project` is refused: the agent works where it is run.
const PROJECT_REFUSED: &str = "`muster mcp add` and `remove` run the agent in the current folder, so they take no --project; run them in the project's folder";
/// What to do with the printed entry in Zed, which has no command for it.
const ZED_ADD_NOTE: &str = "Zed has no command for this: paste the entry above as a key inside the \"context_servers\" object of Zed's settings.json; see https://zed.dev/docs/ai/mcp";
/// What to delete in Zed to unregister muster.
const ZED_REMOVE_NOTE: &str = "Zed has no command for this: delete the \"muster\" key from the \"context_servers\" object of Zed's settings.json; see https://zed.dev/docs/ai/mcp";
/// What follows the hooks file a hook went into.
const HOOK_ADDED: &str =
    "now starts each session with this project's notes and todos, through the SessionStart hook in";
/// What Codex needs before it runs a new hook.
const CODEX_TRUST_NOTE: &str =
    "Codex runs a new hook only once you trust it: open Codex and review it in /hooks";
/// What precedes the hooks file a hook left.
const HOOK_REMOVED: &str = "took muster's SessionStart hook out of";

/// `muster mcp add`: registers muster's MCP server with `args`' agent
/// through its own CLI, then puts muster's session-start hook into that
/// agent's hooks file; for Zed, prints what to add by hand. A failed
/// registration ends muster as the agent's CLI ended, adding no hook.
///
/// # Errors
/// Returns [`CliError::Usage`] when `project_given`, as the agent works in
/// the current folder, [`CliError::UserScopeOnly`] for a scope other than
/// `user` with an agent other than Claude Code, [`CliError::BinaryGone`]
/// when this binary was replaced or removed, [`CliError::AgentMissing`] or
/// [`CliError::AgentNotRunnable`] when the agent cannot be run, and
/// [`CliError::HookFile`] when its hooks file cannot take the hook.
pub fn add(args: &McpRegistration, project_given: bool) -> Result<(), CliError> {
    let agent = checked(args, project_given)?;
    match agent {
        McpAgent::Zed => {
            let entry = json!({
                COMMAND_KEY: own_binary()?.display().to_string(),
                ARGS_KEY: [MCP_SUBCOMMAND],
            });
            let entry = serde_json::to_string_pretty(&entry).unwrap_or_default();
            emit_line(&format!("\"{MCP_CONNECTION_KEY}\": {entry}"));
            print_note(ZED_ADD_NOTE);
            Ok(())
        },
        McpAgent::Claude => {
            let scope = args.scope().unwrap_or(ADD_SCOPE);
            // A project's shared files name the bare `muster` for each
            // person's own `PATH` to find.
            let binary = if scope == McpScope::Project {
                PathBuf::from(APP_NAME)
            } else {
                own_binary()?
            };
            let hooks = claude_settings(scope, &cwd()?)?;
            let words = claude_add_words(scope, &binary);
            // An entry already in the scope is replaced, so adding again
            // brings an older registration up to this one.
            agent_status(agent, &remove_words(agent, Some(scope)), &words, true)?;
            let status = agent_status(agent, &words, &words, false)?;
            hooked(agent, status, &hooks, &binary)
        },
        McpAgent::Codex => {
            let binary = own_binary()?;
            let hooks = codex_hooks()?;
            let words = codex_add_words(&binary);
            let status = agent_status(agent, &words, &words, false)?;
            hooked(agent, status, &hooks, &binary)?;
            print_note(CODEX_TRUST_NOTE);
            Ok(())
        },
    }
}

/// `muster mcp remove`: takes muster's session-start hook out of `args`'
/// agent's hooks files, then replaces muster with the agent's CLI
/// unregistering the server; for Zed, says what to delete by hand. Returns
/// only for Zed or on a failure.
///
/// # Errors
/// Returns what [`add`] returns, but [`CliError::BinaryGone`].
pub fn remove(args: &McpRegistration, project_given: bool) -> Result<(), CliError> {
    let agent = checked(args, project_given)?;
    let files = match agent {
        McpAgent::Zed => {
            print_note(ZED_REMOVE_NOTE);
            return Ok(());
        },
        McpAgent::Claude => {
            let cwd = cwd()?;
            match args.scope() {
                Some(scope) => vec![claude_settings(scope, &cwd)?],
                None => McpScope::iter()
                    .map(|scope| claude_settings(scope, &cwd))
                    .collect::<Result<_, _>>()?,
            }
        },
        McpAgent::Codex => vec![codex_hooks()?],
    };
    for file in &files {
        if file.uninstall()? {
            print_note(&format!("{HOOK_REMOVED} {}", file.path().display()));
        }
    }
    let program = program(agent);
    let words = remove_words(agent, args.scope());
    let source = Command::new(program).args(&words).exec();
    Err(run_failure(program, source, &words))
}

/// The agent `args` names, once the request is one muster takes.
///
/// # Errors
/// Returns [`CliError::Usage`] when `project_given`, and
/// [`CliError::UserScopeOnly`] for a scope other than `user` with an agent
/// other than Claude Code.
fn checked(args: &McpRegistration, project_given: bool) -> Result<McpAgent, CliError> {
    if project_given {
        return Err(CliError::Usage(PROJECT_REFUSED.to_owned()));
    }
    let agent = args.agent();
    if agent != McpAgent::Claude && args.scope().is_some_and(|scope| scope != McpScope::User) {
        return Err(CliError::UserScopeOnly(
            agent.get_message().unwrap_or_default(),
        ));
    }
    Ok(agent)
}

/// Puts the hook running `binary`'s `mcp context` into `hooks` once the
/// agent's registration ended in `status`, saying where; a registration
/// that failed ends muster as it did.
///
/// # Errors
/// Returns [`CliError::HookFile`] when the file cannot take the hook.
fn hooked(
    agent: McpAgent,
    status: ExitStatus,
    hooks: &HookFile,
    binary: &Path,
) -> Result<(), CliError> {
    if !status.success() {
        end_as(status);
    }
    hooks.install(&context_command(binary))?;
    print_note(&format!(
        "{} {HOOK_ADDED} {}",
        agent.get_message().unwrap_or_default(),
        hooks.path().display()
    ));
    Ok(())
}

/// Runs `agent`'s CLI with `words` and waits for it, its output shown, or
/// with `quiet` hidden. A failure to start names `shown`, the command a
/// person runs once the agent is installed.
///
/// # Errors
/// Returns [`CliError::AgentMissing`] or [`CliError::AgentNotRunnable`]
/// when the agent cannot be run.
fn agent_status(
    agent: McpAgent,
    words: &[OsString],
    shown: &[OsString],
    quiet: bool,
) -> Result<ExitStatus, CliError> {
    let program = program(agent);
    let mut command = Command::new(program);
    command.args(words);
    if quiet {
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
    }
    command
        .status()
        .map_err(|source| run_failure(program, source, shown))
}

/// The failure of starting `program` with `words`: missing, with the
/// command to run once it is installed, or not runnable.
fn run_failure(program: &'static str, source: io::Error, words: &[OsString]) -> CliError {
    if source.kind() == ErrorKind::NotFound {
        let command = shell_words(program, words);
        CliError::AgentMissing {
            program,
            source,
            command,
        }
    } else {
        CliError::AgentNotRunnable { program, source }
    }
}

/// Ends muster as a child that ended in `status` did: by its signal, so
/// muster's parent sees the signal, or with its exit code.
fn end_as(status: ExitStatus) -> ! {
    if let Some(signal) = status.signal() {
        // Should the signal not end muster, the code below still fails.
        let _ = emulate_default_handler(signal);
    }
    exit(status.code().unwrap_or(1))
}

/// The binary of `agent`'s own CLI; Zed, which has none, never gets here.
fn program(agent: McpAgent) -> &'static str {
    agent.get_str(PROGRAM).unwrap_or_default()
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

/// The folder muster runs in.
///
/// # Errors
/// Returns [`CliError::CurrentDir`] when it cannot be read.
fn cwd() -> Result<PathBuf, CliError> {
    env::current_dir().map_err(CliError::CurrentDir)
}

/// The folder an environment variable `name` names, when set and not empty.
fn env_dir(name: &str) -> Option<PathBuf> {
    env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

/// The Claude Code settings file of `scope`, in `cwd` for a project's.
///
/// # Errors
/// Returns [`HookFileError::NoHome`] for the user's when no home folder is
/// known and `CLAUDE_CONFIG_DIR` is unset.
fn claude_settings(scope: McpScope, cwd: &Path) -> Result<HookFile, HookFileError> {
    let path = match scope {
        McpScope::User => env_dir(CLAUDE_CONFIG_ENV)
            .or_else(|| home_dir().map(|home| home.join(CLAUDE_DIR)))
            .ok_or(HookFileError::NoHome(CLAUDE_FOLDER))?
            .join(CLAUDE_SETTINGS),
        McpScope::Project => cwd.join(CLAUDE_DIR).join(CLAUDE_SETTINGS),
        McpScope::Local => cwd.join(CLAUDE_DIR).join(CLAUDE_LOCAL_SETTINGS),
    };
    Ok(HookFile::builder().path(path).build())
}

/// Codex's hooks file.
///
/// # Errors
/// Returns [`HookFileError::NoHome`] when no home folder is known and
/// `CODEX_HOME` is unset.
fn codex_hooks() -> Result<HookFile, HookFileError> {
    let path = env_dir(CODEX_HOME_ENV)
        .or_else(|| home_dir().map(|home| home.join(CODEX_DIR)))
        .ok_or(HookFileError::NoHome(CODEX_FOLDER))?
        .join(CODEX_HOOKS);
    Ok(HookFile::builder().path(path).build())
}

/// The arguments of Claude Code's command registering `binary` in `scope`,
/// its tools loaded up front.
fn claude_add_words(scope: McpScope, binary: &Path) -> Vec<OsString> {
    let entry = json!({
        TYPE_KEY: STDIO_TYPE,
        COMMAND_KEY: binary.to_string_lossy(),
        ARGS_KEY: [MCP_SUBCOMMAND],
        ALWAYS_LOAD_KEY: true,
    });
    [
        AGENT_MCP,
        ADD_JSON_VERB,
        SCOPE_FLAG,
        &scope.to_string(),
        MCP_CONNECTION_KEY,
        &entry.to_string(),
    ]
    .into_iter()
    .map(OsString::from)
    .collect()
}

/// The arguments of Codex's command registering `binary`.
fn codex_add_words(binary: &Path) -> Vec<OsString> {
    let mut words: Vec<OsString> = [AGENT_MCP, ADD_VERB, MCP_CONNECTION_KEY, END_OF_OPTIONS]
        .into_iter()
        .map(OsString::from)
        .collect();
    words.extend([binary.into(), MCP_SUBCOMMAND.into()]);
    words
}

/// The arguments of `agent`'s command unregistering muster, from `scope`
/// when Claude Code is given one; Codex's CLI has no scopes.
fn remove_words(agent: McpAgent, scope: Option<McpScope>) -> Vec<OsString> {
    let mut words: Vec<OsString> = vec![AGENT_MCP.into(), REMOVE_VERB.into()];
    if agent == McpAgent::Claude
        && let Some(scope) = scope
    {
        words.extend([SCOPE_FLAG.into(), scope.to_string().into()]);
    }
    words.push(MCP_CONNECTION_KEY.into());
    words
}

/// `program` and `words` as a person would type them.
fn shell_words(program: &str, words: &[OsString]) -> String {
    let words: Vec<String> = iter::once(program.to_owned())
        .chain(
            words
                .iter()
                .map(|word| OsStr::to_string_lossy(word).into_owned()),
        )
        .collect();
    shlex::try_join(words.iter().map(String::as_str)).unwrap_or_else(|_| words.join(WORD_SEPARATOR))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Claude Code is given muster's entry as JSON with `alwaysLoad`, in the
    /// scope asked for.
    #[test]
    fn claude_registers_an_always_loaded_entry() {
        let words = claude_add_words(McpScope::Local, Path::new("/opt/bin/muster"));

        assert_eq!(words[..5], [
            "mcp", "add-json", "--scope", "local", "muster"
        ]);
        let entry: serde_json::Value = serde_json::from_str(words[5].to_str().unwrap()).unwrap();
        assert_eq!(
            entry,
            json!({"type": "stdio", "command": "/opt/bin/muster", "args": ["mcp"], "alwaysLoad": true})
        );
    }

    /// Each scope's settings file is the one Claude Code reads for it.
    #[test]
    fn each_scope_has_its_settings_file() {
        let cwd = Path::new("/work/shop");

        assert_eq!(
            claude_settings(McpScope::Project, cwd).unwrap().path(),
            Path::new("/work/shop/.claude/settings.json")
        );
        assert_eq!(
            claude_settings(McpScope::Local, cwd).unwrap().path(),
            Path::new("/work/shop/.claude/settings.local.json")
        );
    }
}
