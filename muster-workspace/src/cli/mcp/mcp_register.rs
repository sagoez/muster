//! `muster mcp add` and `muster mcp remove`: the agent's own CLI registering
//! or unregistering muster's MCP server, its output and failure passed
//! through, and muster's session-start hook put into or taken out of the
//! agent's hooks file. Every file is checked before anything changes, and a
//! step that fails puts back what the steps before it changed.

use std::{
    env,
    ffi::OsString,
    io::{self, ErrorKind},
    os::unix::process::ExitStatusExt,
    path::PathBuf,
    process::{Command, ExitStatus, Stdio, exit},
};

use muster_client::{runnable, this_binary};
use muster_domain::constants::{APP_NAME, MCP_CONNECTION_KEY, MCP_SUBCOMMAND};
use muster_os::path::home_dir;
use serde_json::{Map, Value, json};
use signal_hook::low_level::emulate_default_handler;
use strum::{EnumMessage, EnumProperty};

use crate::cli::{
    failure::error::CliError,
    mcp::{
        agent_keys::{
            ALWAYS_LOAD_KEY, ARGS_KEY, COMMAND_KEY, ENV_KEY, STDIO_TYPE, TRANSPORT_KEY, TYPE_KEY,
        },
        agent_output::AgentOutput,
        claude_config::ClaudeConfig,
        hook_command::context_command,
        hook_file::HookFile,
        hook_file_error::HookFileError,
        mcp_agent::{McpAgent, PROGRAM},
        mcp_registration::McpRegistration,
        mcp_scope::McpScope,
    },
    present::emit::emit_line,
    rows::report::print_note,
    stack::shell_join,
};

/// The scope an add registers in when none is given: every project, as the
/// server finds each call's project itself.
const ADD_SCOPE: McpScope = McpScope::User;
/// The agents' subcommand for MCP servers.
const AGENT_MCP: &str = "mcp";
/// Codex's verb that registers a server, replacing one of the same name.
const ADD_VERB: &str = "add";
/// Claude Code's verb that registers a server from its JSON entry, which
/// alone carries `alwaysLoad`.
const ADD_JSON_VERB: &str = "add-json";
/// The agents' verb that unregisters one.
const REMOVE_VERB: &str = "remove";
/// Codex's verb that shows one server.
const GET_VERB: &str = "get";
/// Asks Codex for a server as JSON.
const JSON_FLAG: &str = "--json";
/// Claude Code's flag naming the scope.
const SCOPE_FLAG: &str = "--scope";
/// Codex's flag setting one of the server's environment variables.
const ENV_FLAG: &str = "--env";
/// Between a variable's name and value on Codex's command line.
const ENV_SEPARATOR: &str = "=";
/// Ends the agent's own options; the server's command line follows.
const END_OF_OPTIONS: &str = "--";
/// The exit muster takes for an agent's CLI that ended with neither a code
/// nor a signal muster could raise.
const AGENT_FAILED: i32 = 1;
/// The variable that moves Codex's home folder off `~/.codex`.
const CODEX_HOME_ENV: &str = "CODEX_HOME";
/// Codex's home folder, in the home folder and in a project.
const CODEX_DIR: &str = ".codex";
/// Codex's hooks file.
const CODEX_HOOKS: &str = "hooks.json";
/// Why `--project` is refused: the agent works where it is run.
const PROJECT_REFUSED: &str = "`muster mcp add` and `remove` run the agent in the current folder, so they take no --project; run them in the project's folder";
/// What to do with the printed entry in Zed, which has no command for it.
const ZED_ADD_NOTE: &str = "Zed has no command for this: paste the entry above as a key inside the \"context_servers\" object of Zed's settings.json; see https://zed.dev/docs/ai/mcp";
/// What to delete in Zed to unregister muster.
const ZED_REMOVE_NOTE: &str = "Zed has no command for this: delete the \"muster\" key from the \"context_servers\" object of Zed's settings.json; see https://zed.dev/docs/ai/mcp";
/// What follows the agent's name and precedes the file its hook went in.
const HOOK_ADDED: &str =
    "now starts each session with this project's notes and todos, through the SessionStart hook in";
/// What precedes the file whose hook already serves sessions here.
const HOOK_ALREADY: &str =
    "added no second SessionStart hook: sessions here already run muster's from";
/// What Codex needs before it runs a new hook.
const CODEX_TRUST_NOTE: &str =
    "Codex runs a new hook only once you trust it: open Codex and review it in /hooks";
/// What precedes the file a hook left.
const HOOK_REMOVED: &str = "took muster's SessionStart hook out of";
/// What precedes the scope whose entry Claude Code uses here over the new one.
const SHADOWED_LEAD: &str = "Claude Code uses muster's";
/// Between that scope and the one just registered.
const SHADOWED_MIDDLE: &str = "entry in this folder over the";
/// Follows the scope just registered, before the command that removes the
/// one that wins.
const SHADOWED_TAIL: &str = "one; to use the new one here, run: muster mcp remove claude --scope";

/// `muster mcp add`: checks the agent's hooks file, registers muster's MCP
/// server with `args`' agent through its own CLI, then puts muster's
/// session-start hook into that file; for Zed, prints what to add by hand.
/// A registration that fails puts back the entry it replaced and ends muster
/// as the agent's CLI ended; a hook that cannot be written puts back the
/// registration too.
///
/// # Errors
/// Returns [`CliError::Usage`] when `project_given`, as the agent works in
/// the current folder, [`CliError::UserScopeOnly`] for a scope other than
/// `user` with an agent other than Claude Code, [`CliError::BinaryGone`]
/// when this binary was replaced or removed, [`CliError::UnrepresentablePath`]
/// for a binary path that is not UTF-8, [`CliError::AgentMissing`] or
/// [`CliError::AgentNotRunnable`] when the agent cannot be run, and
/// [`CliError::HookFile`] when a file cannot be read or take the hook; in
/// every case nothing has changed.
pub fn add(args: &McpRegistration, project_given: bool) -> Result<(), CliError> {
    match checked(args, project_given)? {
        McpAgent::Zed => {
            let entry = json!({ COMMAND_KEY: own_binary()?, ARGS_KEY: [MCP_SUBCOMMAND] });
            let entry = serde_json::to_string_pretty(&entry).unwrap_or_default();
            emit_line(&format!("\"{MCP_CONNECTION_KEY}\": {entry}"));
            print_note(ZED_ADD_NOTE);
            Ok(())
        },
        McpAgent::Claude => add_claude(args.scope().unwrap_or(ADD_SCOPE)),
        McpAgent::Codex => add_codex(),
    }
}

/// `muster mcp remove`: checks the hooks files the removal may touch, runs
/// the agent's CLI unregistering muster, and once it succeeds takes muster's
/// hook out of the files of the scopes it removed muster from, and no
/// other; for Zed, says what to delete by hand. A removal that fails ends
/// muster as the agent's CLI ended, with nothing changed.
///
/// # Errors
/// Returns what [`add`] returns, but [`CliError::BinaryGone`].
pub fn remove(args: &McpRegistration, project_given: bool) -> Result<(), CliError> {
    match checked(args, project_given)? {
        McpAgent::Zed => {
            print_note(ZED_REMOVE_NOTE);
            Ok(())
        },
        McpAgent::Claude => remove_claude(args.scope()),
        McpAgent::Codex => {
            let hooks = codex_hooks()?;
            hooks.check_uninstall()?;
            let words = remove_words(McpAgent::Codex, None);
            succeeded(agent_status(
                McpAgent::Codex,
                &words,
                &words,
                AgentOutput::Shown,
            )?);
            unhooked(&hooks)
        },
    }
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

/// Registers muster with Claude Code in `scope` and adds its hook, as
/// [`add`] describes. A project's shared files name the bare `muster`,
/// found on each person's own `PATH`, and its hook runs only where muster
/// is installed. When another settings file a session here reads already
/// runs muster's hook, none is added, so a session gets it once.
///
/// # Errors
/// Returns what [`add`] returns.
fn add_claude(scope: McpScope) -> Result<(), CliError> {
    let shared = scope == McpScope::Project;
    let binary = if shared {
        APP_NAME.to_owned()
    } else {
        own_binary()?
    };
    let command = context_command(&binary, shared)?;
    let config = ClaudeConfig::find(cwd()?)?;
    let target = config.settings(scope);
    let elsewhere = if target.has_hook()? {
        None
    } else {
        hooked_elsewhere(config.other_settings(scope))
    };
    if elsewhere.is_none() {
        target.check_install(&command)?;
    }
    let existing = config.entry(scope)?;
    let entry = server_entry(existing.as_ref(), &binary);
    let changed = register_claude(scope, existing.as_ref(), &entry)?;
    match &elsewhere {
        Some(file) => print_note(&format!("{HOOK_ALREADY} {}", file.path().display())),
        None => {
            if let Err(error) = target.install(&command) {
                if changed {
                    restore_claude(scope, existing.as_ref());
                }
                return Err(error.into());
            }
            hook_added(McpAgent::Claude, &target);
        },
    }
    if let Some(winner) = config.scopes_holding()?.first()
        && *winner != scope
    {
        print_note(&format!(
            "{SHADOWED_LEAD} {winner} {SHADOWED_MIDDLE} {scope} {SHADOWED_TAIL} {winner}"
        ));
    }
    Ok(())
}

/// Makes `entry` muster's Claude Code server in `scope`, where `existing`
/// was. Returns whether it changed anything: not when `existing` is already
/// `entry`. The old entry is removed only right before the new one is
/// added, and put back when the add fails, which ends muster as the agent's
/// CLI ended.
///
/// # Errors
/// Returns [`CliError::AgentMissing`] or [`CliError::AgentNotRunnable`]
/// when Claude Code cannot be run.
fn register_claude(
    scope: McpScope,
    existing: Option<&Value>,
    entry: &Value,
) -> Result<bool, CliError> {
    if existing == Some(entry) {
        return Ok(false);
    }
    let words = claude_add_words(scope, entry);
    if existing.is_some() {
        let removing = remove_words(McpAgent::Claude, Some(scope));
        succeeded(agent_status(
            McpAgent::Claude,
            &removing,
            &words,
            AgentOutput::Hidden,
        )?);
    }
    let added = agent_status(McpAgent::Claude, &words, &words, AgentOutput::Shown);
    if !added.as_ref().is_ok_and(ExitStatus::success) {
        if let Some(previous) = existing {
            let _ = put_back_claude(scope, previous);
        }
        succeeded(added?);
    }
    Ok(true)
}

/// Puts `scope`'s registration back as it was before muster changed it:
/// `existing` again, or none.
fn restore_claude(scope: McpScope, existing: Option<&Value>) {
    let removing = remove_words(McpAgent::Claude, Some(scope));
    let _ = agent_status(McpAgent::Claude, &removing, &removing, AgentOutput::Hidden);
    if let Some(previous) = existing {
        let _ = put_back_claude(scope, previous);
    }
}

/// Registers `previous` in `scope` again, quietly.
///
/// # Errors
/// Returns the failure of starting Claude Code.
fn put_back_claude(scope: McpScope, previous: &Value) -> Result<ExitStatus, CliError> {
    let words = claude_add_words(scope, previous);
    agent_status(McpAgent::Claude, &words, &words, AgentOutput::Hidden)
}

/// Unregisters muster from Claude Code, from `scope` or, without one, from
/// the one scope holding it, and takes muster's hook out of the settings
/// files of only the scopes it was removed from.
///
/// # Errors
/// Returns what [`remove`] returns.
fn remove_claude(scope: Option<McpScope>) -> Result<(), CliError> {
    let config = ClaudeConfig::find(cwd()?)?;
    let before = config.scopes_holding()?;
    let candidates = scope.map_or_else(|| before.clone(), |scope| vec![scope]);
    for candidate in &candidates {
        config.settings(*candidate).check_uninstall()?;
    }
    let words = remove_words(McpAgent::Claude, scope);
    succeeded(agent_status(
        McpAgent::Claude,
        &words,
        &words,
        AgentOutput::Shown,
    )?);
    let removed = match scope {
        Some(scope) => vec![scope],
        None => {
            let after = config.scopes_holding()?;
            before
                .into_iter()
                .filter(|scope| !after.contains(scope))
                .collect()
        },
    };
    for scope in removed {
        unhooked(&config.settings(scope))?;
    }
    Ok(())
}

/// Registers muster with Codex, keeping the arguments and environment an
/// entry already there was given, and adds its hook to Codex's hooks file
/// unless the project's own already runs it.
///
/// # Errors
/// Returns what [`add`] returns.
fn add_codex() -> Result<(), CliError> {
    let binary = own_binary()?;
    let command = context_command(&binary, false)?;
    let hooks = codex_hooks()?;
    let project = HookFile::builder()
        .path(cwd()?.join(CODEX_DIR).join(CODEX_HOOKS))
        .build();
    let elsewhere = if hooks.has_hook()? || project.path() == hooks.path() {
        None
    } else {
        hooked_elsewhere(vec![project])
    };
    if elsewhere.is_none() {
        hooks.check_install(&command)?;
    }
    let existing = codex_entry()?;
    let words = codex_add_words(&binary, existing.as_ref());
    succeeded(agent_status(
        McpAgent::Codex,
        &words,
        &words,
        AgentOutput::Shown,
    )?);
    match &elsewhere {
        Some(file) => print_note(&format!("{HOOK_ALREADY} {}", file.path().display())),
        None => {
            if let Err(error) = hooks.install(&command) {
                let undo = match &existing {
                    Some(previous) => codex_restore_words(previous),
                    None => remove_words(McpAgent::Codex, None),
                };
                let _ = agent_status(McpAgent::Codex, &undo, &undo, AgentOutput::Hidden);
                return Err(error.into());
            }
            hook_added(McpAgent::Codex, &hooks);
            print_note(CODEX_TRUST_NOTE);
        },
    }
    Ok(())
}

/// The transport of muster's Codex server, as `codex mcp get --json`
/// prints it; none when Codex has no such server or prints no JSON.
///
/// # Errors
/// Returns [`CliError::AgentMissing`] or [`CliError::AgentNotRunnable`]
/// when Codex cannot be run.
fn codex_entry() -> Result<Option<Value>, CliError> {
    let program = program(McpAgent::Codex);
    let words: Vec<OsString> = [AGENT_MCP, GET_VERB, MCP_CONNECTION_KEY, JSON_FLAG]
        .into_iter()
        .map(OsString::from)
        .collect();
    let output = Command::new(program)
        .args(&words)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(|source| run_failure(program, source, &words))?;
    if !output.status.success() {
        return Ok(None);
    }
    Ok(serde_json::from_slice::<Value>(&output.stdout)
        .ok()
        .and_then(|server| server.get(TRANSPORT_KEY).cloned()))
}

/// The first of `files` that holds muster's hook; a file that cannot be
/// read counts as holding none, as muster does not change it.
fn hooked_elsewhere(files: Vec<HookFile>) -> Option<HookFile> {
    files
        .into_iter()
        .find(|file| file.has_hook().unwrap_or(false))
}

/// Says that `agent` now runs the hook from `file`.
fn hook_added(agent: McpAgent, file: &HookFile) {
    print_note(&format!(
        "{} {HOOK_ADDED} {}",
        agent.get_message().unwrap_or_default(),
        file.path().display()
    ));
}

/// Takes muster's hook out of `file`, saying so when it was there.
///
/// # Errors
/// Returns [`CliError::HookFile`] when the file cannot give it up.
fn unhooked(file: &HookFile) -> Result<(), CliError> {
    if file.uninstall()? {
        print_note(&format!("{HOOK_REMOVED} {}", file.path().display()));
    }
    Ok(())
}

/// Runs `agent`'s CLI with `words` and waits for it, its output shown or
/// hidden as `output` says. A failure to start names `shown`, the command
/// a person runs once the agent is installed.
///
/// # Errors
/// Returns [`CliError::AgentMissing`] or [`CliError::AgentNotRunnable`]
/// when the agent cannot be run.
fn agent_status(
    agent: McpAgent,
    words: &[OsString],
    shown: &[OsString],
    output: AgentOutput,
) -> Result<ExitStatus, CliError> {
    let program = program(agent);
    let mut command = Command::new(program);
    command.args(words);
    if output == AgentOutput::Hidden {
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
    if source.kind() != ErrorKind::NotFound {
        return CliError::AgentNotRunnable { program, source };
    }
    let words: Vec<String> = words
        .iter()
        .map(|word| word.to_string_lossy().into_owned())
        .collect();
    match shell_join(
        [program]
            .into_iter()
            .chain(words.iter().map(String::as_str)),
    ) {
        Ok(command) => CliError::AgentMissing {
            program,
            source,
            command,
        },
        Err(error) => error,
    }
}

/// Returns when `status` is a success; otherwise ends muster as the child
/// that ended in it did: by its signal, so muster's parent sees the signal,
/// or with its exit code.
fn succeeded(status: ExitStatus) {
    if status.success() {
        return;
    }
    if let Some(signal) = status.signal() {
        // Should the signal not end muster, the exit below still fails.
        let _ = emulate_default_handler(signal);
    }
    exit(status.code().unwrap_or(AGENT_FAILED))
}

/// The binary of `agent`'s own CLI; Zed, which has none, never gets here.
fn program(agent: McpAgent) -> &'static str {
    agent.get_str(PROGRAM).unwrap_or_default()
}

/// This binary's own path, as a config names it.
///
/// # Errors
/// Returns [`CliError::BinaryGone`] when the binary was replaced or removed
/// since it started and nothing runnable is left at its path, and
/// [`CliError::UnrepresentablePath`] for a path that is not UTF-8, which a
/// config could not name faithfully.
fn own_binary() -> Result<String, CliError> {
    let binary = this_binary();
    runnable(&binary)
        .ok_or(CliError::BinaryGone(binary))?
        .into_os_string()
        .into_string()
        .map_err(|path| CliError::UnrepresentablePath(PathBuf::from(path)))
}

/// The folder muster runs in.
///
/// # Errors
/// Returns [`CliError::CurrentDir`] when it cannot be read.
fn cwd() -> Result<PathBuf, CliError> {
    env::current_dir().map_err(CliError::CurrentDir)
}

/// Codex's hooks file.
///
/// # Errors
/// Returns [`HookFileError::NoHome`] when no home folder is known and
/// `CODEX_HOME` is unset.
fn codex_hooks() -> Result<HookFile, HookFileError> {
    let path = env::var_os(CODEX_HOME_ENV)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| home_dir().map(|home| home.join(CODEX_DIR)))
        .ok_or(HookFileError::NoHome(McpAgent::Codex))?
        .join(CODEX_HOOKS);
    Ok(HookFile::builder().path(path).build())
}

/// Whether `args` runs muster's server, so it can be kept as given.
fn serves_mcp(args: &Value) -> bool {
    args.as_array()
        .and_then(|args| args.first())
        .is_some_and(|first| first == MCP_SUBCOMMAND)
}

/// muster's Claude Code entry running `binary`, its tools loaded up front:
/// `existing`, when there is one, with its other keys, its environment and
/// its arguments (such as `--as NAME`) kept.
fn server_entry(existing: Option<&Value>, binary: &str) -> Value {
    let mut entry: Map<String, Value> = existing
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let args = entry
        .get(ARGS_KEY)
        .filter(|args| serves_mcp(args))
        .cloned()
        .unwrap_or_else(|| json!([MCP_SUBCOMMAND]));
    entry.insert(TYPE_KEY.to_owned(), STDIO_TYPE.into());
    entry.insert(COMMAND_KEY.to_owned(), binary.into());
    entry.insert(ARGS_KEY.to_owned(), args);
    entry.insert(ALWAYS_LOAD_KEY.to_owned(), true.into());
    Value::Object(entry)
}

/// The arguments of Claude Code's command registering `entry` in `scope`.
fn claude_add_words(scope: McpScope, entry: &Value) -> Vec<OsString> {
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

/// The strings of `value`'s list, none for anything else.
fn strings(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// The arguments of Codex's command registering `command` with `args` and
/// the environment of `transport`.
fn codex_words(command: &str, args: &[String], transport: Option<&Value>) -> Vec<OsString> {
    let mut words: Vec<OsString> =
        vec![AGENT_MCP.into(), ADD_VERB.into(), MCP_CONNECTION_KEY.into()];
    if let Some(env) = transport
        .and_then(|transport| transport.get(ENV_KEY))
        .and_then(Value::as_object)
    {
        for (name, value) in env {
            if let Some(value) = value.as_str() {
                words.extend([
                    ENV_FLAG.into(),
                    format!("{name}{ENV_SEPARATOR}{value}").into(),
                ]);
            }
        }
    }
    words.push(END_OF_OPTIONS.into());
    words.push(command.into());
    words.extend(args.iter().map(OsString::from));
    words
}

/// The arguments of Codex's command registering `binary`, keeping the
/// arguments and environment `existing`, the transport of an entry already
/// there, was given.
fn codex_add_words(binary: &str, existing: Option<&Value>) -> Vec<OsString> {
    let kept = existing
        .and_then(|transport| transport.get(ARGS_KEY))
        .filter(|args| serves_mcp(args));
    let args = match kept {
        Some(args) => strings(Some(args)),
        None => vec![MCP_SUBCOMMAND.to_owned()],
    };
    codex_words(binary, &args, existing)
}

/// The arguments of Codex's command registering `previous`, a transport as
/// it was, again.
fn codex_restore_words(previous: &Value) -> Vec<OsString> {
    let command = previous
        .get(COMMAND_KEY)
        .and_then(Value::as_str)
        .unwrap_or(APP_NAME);
    codex_words(command, &strings(previous.get(ARGS_KEY)), Some(previous))
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The words, as text.
    fn texts(words: &[OsString]) -> Vec<String> {
        words
            .iter()
            .map(|word| word.to_string_lossy().into_owned())
            .collect()
    }

    /// A new entry runs the binary as muster's server with its tools loaded
    /// up front, and Claude Code is given it as JSON in the scope asked for.
    #[test]
    fn claude_registers_an_always_loaded_entry() {
        let entry = server_entry(None, "/opt/bin/muster");
        let words = claude_add_words(McpScope::Local, &entry);

        assert_eq!(texts(&words)[..5], [
            "mcp", "add-json", "--scope", "local", "muster"
        ]);
        assert_eq!(
            serde_json::from_str::<Value>(&texts(&words)[5]).unwrap(),
            json!({"type": "stdio", "command": "/opt/bin/muster", "args": ["mcp"], "alwaysLoad": true})
        );
    }

    /// Adding again keeps an entry's `--as NAME`, its environment and its
    /// other keys, and replaces only the binary.
    #[test]
    fn a_customised_entry_keeps_its_arguments_and_environment() {
        let existing = json!({
            "type": "stdio",
            "command": "/old/muster",
            "args": ["mcp", "--as", "reviewer"],
            "env": {"MUSTER_AGENT": "reviewer"},
            "timeout": 30,
        });

        assert_eq!(
            server_entry(Some(&existing), "/opt/bin/muster"),
            json!({
                "type": "stdio",
                "command": "/opt/bin/muster",
                "args": ["mcp", "--as", "reviewer"],
                "env": {"MUSTER_AGENT": "reviewer"},
                "timeout": 30,
                "alwaysLoad": true,
            })
        );
        let unrelated = json!({"command": "x", "args": ["serve"]});
        assert_eq!(
            server_entry(Some(&unrelated), "/opt/bin/muster")["args"],
            json!(["mcp"])
        );
    }

    /// Codex is given the binary with the arguments and environment of the
    /// entry it replaces, and putting that entry back names it as it was.
    #[test]
    fn codex_keeps_a_customised_entry() {
        let transport = json!({
            "type": "stdio",
            "command": "/old/muster",
            "args": ["mcp", "--as", "reviewer"],
            "env": {"A": "1"},
        });

        assert_eq!(texts(&codex_add_words("/opt/bin/muster", None)), [
            "mcp",
            "add",
            "muster",
            "--",
            "/opt/bin/muster",
            "mcp"
        ]);
        assert_eq!(
            texts(&codex_add_words("/opt/bin/muster", Some(&transport))),
            [
                "mcp",
                "add",
                "muster",
                "--env",
                "A=1",
                "--",
                "/opt/bin/muster",
                "mcp",
                "--as",
                "reviewer"
            ]
        );
        assert_eq!(texts(&codex_restore_words(&transport)), [
            "mcp",
            "add",
            "muster",
            "--env",
            "A=1",
            "--",
            "/old/muster",
            "mcp",
            "--as",
            "reviewer"
        ]);
    }
}
