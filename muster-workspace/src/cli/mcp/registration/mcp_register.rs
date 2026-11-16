//! `muster mcp add` and `muster mcp remove`: the agent's own CLI registering
//! or unregistering muster's MCP server, its output and failure passed
//! through, and muster's session-start hook put into or taken out of the
//! hooks file of the scope registered. Only that scope's files are read,
//! each is checked before anything changes, termination signals wait while
//! the config changes, and a step that fails puts back what the steps
//! before it changed.

use std::{
    env,
    ffi::OsString,
    io::{self, ErrorKind},
    os::unix::process::{CommandExt, ExitStatusExt},
    path::PathBuf,
    process::{Command, ExitStatus, Stdio, exit},
};

use muster_client::{runnable, this_binary};
use muster_domain::constants::{APP_NAME, MCP_CONNECTION_KEY, MCP_SUBCOMMAND};
use serde_json::{Map, Value, json};
use signal_hook::low_level::emulate_default_handler;
use strum::{EnumMessage, EnumProperty};

use crate::cli::{
    failure::error::CliError,
    mcp::{
        agent_keys::{ALWAYS_LOAD_KEY, ARGS_KEY, COMMAND_KEY, STDIO_TYPE, TYPE_KEY},
        agent_output::AgentOutput,
        claude_config::ClaudeConfig,
        codex_config::{CodexConfig, codex_entry},
        codex_entry::CodexEntry,
        deferred_signals::DeferredSignals,
        hook_command::context_command,
        hook_file::HookFile,
        mcp_agent::{McpAgent, PROGRAM},
        mcp_registration::McpRegistration,
        mcp_scope::McpScope,
    },
    present::emit::emit_line,
    rows::report::{print_error, print_note},
    stack::shell_join,
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
/// Between the words of a command that cannot be quoted, said only so a
/// person can still read it.
const WORD_SEPARATOR: &str = " ";
/// Between commands a person runs one after the other.
const AND_THEN: &str = " && ";
/// The process group a step muster takes on its own runs in: one of its
/// own, so the Ctrl-C meant for the person's command cannot cut it short.
const OWN_GROUP: i32 = 0;
/// The exit muster takes for an agent's CLI that ended with neither a code
/// nor a signal muster could raise.
const AGENT_FAILED: i32 = 1;
/// Why `--project` is refused: the agent works where it is run.
const PROJECT_REFUSED: &str = "`muster mcp add` and `remove` run the agent in the current folder, so they take no --project; run them in the project's folder";
/// What to do with the printed entry in Zed, which has no command for it.
const ZED_ADD_NOTE: &str = "Zed has no command for this: paste the entry above as a key inside the \"context_servers\" object of Zed's settings.json; see https://zed.dev/docs/ai/mcp";
/// What to delete in Zed to unregister muster.
const ZED_REMOVE_NOTE: &str = "Zed has no command for this: delete the \"muster\" key from the \"context_servers\" object of Zed's settings.json; see https://zed.dev/docs/ai/mcp";
/// What follows the agent's name and precedes the file its hook is in.
const HOOK_ADDED: &str =
    "starts each session with this project's notes and todos, through the SessionStart hook in";
/// What Codex needs before it runs a new hook.
const CODEX_TRUST_NOTE: &str =
    "Codex runs a new hook only once you trust it: open Codex and review it in /hooks";
/// What precedes the file a hook left.
const HOOK_REMOVED: &str = "took muster's SessionStart hook out of";

/// `muster mcp add`: checks the hooks file of the scope registered,
/// registers muster's MCP server with `args`' agent, then puts muster's
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
/// every case the agent's config is as it was.
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
/// muster as the agent's CLI ended, with nothing changed; once it has
/// succeeded, a hook that cannot be taken out is reported and the command
/// still succeeds.
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
            let hooks = CodexConfig::find()?.hooks();
            hooks.check_uninstall()?;
            let words = remove_words(McpAgent::Codex, None);
            let deferred = DeferredSignals::begin();
            succeeded(agent_status(McpAgent::Codex, &words, &words)?);
            unhooked(&hooks);
            drop(deferred);
            Ok(())
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

/// Registers muster with Claude Code in `scope` and adds its hook to that
/// scope's settings file, as [`add`] describes. A project's shared files
/// name the bare `muster`, found on each person's own `PATH`, and its hook
/// runs only where muster is installed.
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
    let hooks = config.settings(scope);
    hooks.check_install(&command)?;
    let existing = config.entry(scope)?;
    let entry = server_entry(existing.as_ref(), &binary);
    let deferred = DeferredSignals::begin();
    let changed = register_claude(scope, existing.as_ref(), &entry)?;
    if let Err(error) = hooks.install(&command) {
        if changed {
            restore_claude(scope, existing.as_ref());
        }
        return Err(error.into());
    }
    drop(deferred);
    hook_added(McpAgent::Claude, &hooks);
    Ok(())
}

/// Makes `entry` muster's Claude Code server in `scope`, where `existing`
/// was. Returns whether it changed anything: not when `existing` is already
/// `entry`. The old entry is removed only right before the new one is
/// added, and put back when the add fails, which then ends muster as the
/// agent's CLI ended; a removal the agent refuses says why and ends it the
/// same way, with nothing changed.
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
        let (status, said) = hidden_run(McpAgent::Claude, &removing, &words)?;
        if !status.success() {
            print_error(&CliError::EntryNotReplaced(said));
            succeeded(status);
        }
    }
    let added = agent_status(McpAgent::Claude, &words, &words);
    if !added.as_ref().is_ok_and(ExitStatus::success) {
        if let Some(previous) = existing {
            put_back(McpAgent::Claude, &claude_add_words(scope, previous));
        }
        succeeded(added?);
    }
    Ok(true)
}

/// Puts `scope`'s registration back as it was before muster changed it:
/// `existing` again, or none.
fn restore_claude(scope: McpScope, existing: Option<&Value>) {
    let steps = restoring_words(scope, existing);
    let Some((removing, rest)) = steps.split_first() else {
        return;
    };
    let removed =
        hidden_run(McpAgent::Claude, removing, removing).is_ok_and(|(status, _)| status.success());
    if !removed {
        report_unrestored(McpAgent::Claude, &steps);
        return;
    }
    for step in rest {
        put_back(McpAgent::Claude, step);
    }
}

/// The steps that put `scope`'s Claude Code registration back as it was:
/// removing what muster added, then adding `existing` again when there was
/// one.
fn restoring_words(scope: McpScope, existing: Option<&Value>) -> Vec<Vec<OsString>> {
    let mut steps = vec![remove_words(McpAgent::Claude, Some(scope))];
    steps.extend(existing.map(|previous| claude_add_words(scope, previous)));
    steps
}

/// Runs `agent` with `words`, a step that puts its config back, on its own;
/// when it fails, says loudly how to do it by hand, never dropping it.
fn put_back(agent: McpAgent, words: &[OsString]) {
    let done = hidden_run(agent, words, words).is_ok_and(|(status, _)| status.success());
    if !done {
        report_unrestored(agent, &[words.to_vec()]);
    }
}

/// Says that `agent` run with each of `steps`, in order, must be done by
/// hand to put its config back as it was.
fn report_unrestored(agent: McpAgent, steps: &[Vec<OsString>]) {
    print_error(&CliError::EntryNotRestored(typed_commands(agent, steps)));
}

/// `agent` run with each of `steps`, as a person types them one after the
/// other.
fn typed_commands(agent: McpAgent, steps: &[Vec<OsString>]) -> String {
    steps
        .iter()
        .map(|words| typed_command(program(agent), words))
        .collect::<Vec<_>>()
        .join(AND_THEN)
}

/// `program` with `words` as a person types it; a word no shell could
/// carry, which no path or entry holds, is shown as it is.
fn typed_command(program: &str, words: &[OsString]) -> String {
    let words: Vec<String> = words
        .iter()
        .map(|word| word.to_string_lossy().into_owned())
        .collect();
    let all = || {
        [program]
            .into_iter()
            .chain(words.iter().map(String::as_str))
    };
    shell_join(all()).unwrap_or_else(|_| all().collect::<Vec<_>>().join(WORD_SEPARATOR))
}

/// Unregisters muster from Claude Code, from `scope` or, without one, from
/// the one scope holding it, and takes muster's hook out of the settings
/// files of only the scopes it was removed from. With `scope`, no other
/// scope's file is read.
///
/// # Errors
/// Returns what [`remove`] returns.
fn remove_claude(scope: Option<McpScope>) -> Result<(), CliError> {
    let config = ClaudeConfig::find(cwd()?)?;
    let words = remove_words(McpAgent::Claude, scope);
    if let Some(scope) = scope {
        let hooks = config.settings(scope);
        hooks.check_uninstall()?;
        let deferred = DeferredSignals::begin();
        succeeded(agent_status(McpAgent::Claude, &words, &words)?);
        unhooked(&hooks);
        drop(deferred);
        return Ok(());
    }
    let before = config.scopes_holding()?;
    for holding in &before {
        config.settings(*holding).check_uninstall()?;
    }
    let deferred = DeferredSignals::begin();
    succeeded(agent_status(McpAgent::Claude, &words, &words)?);
    match config.scopes_holding() {
        Ok(after) => before
            .into_iter()
            .filter(|holding| !after.contains(holding))
            .for_each(|removed| unhooked(&config.settings(removed))),
        Err(error) => print_error(&CliError::ScopeUnknown(error)),
    }
    drop(deferred);
    Ok(())
}

/// Registers muster with Codex and adds its hook to Codex's hooks file. An
/// entry Codex already has is pointed at this binary in its `config.toml`,
/// in place, every other key of it kept and none passed on a command line;
/// without one, `codex mcp add` makes it.
///
/// # Errors
/// Returns what [`add`] returns.
fn add_codex() -> Result<(), CliError> {
    let binary = own_binary()?;
    let command = context_command(&binary, false)?;
    let config = CodexConfig::find()?;
    let hooks = config.hooks();
    hooks.check_install(&command)?;
    let original = config.config_text()?;
    let entry = codex_entry(original.as_deref(), &binary, &config.config_path())?;
    let deferred = DeferredSignals::begin();
    match &entry {
        CodexEntry::Absent => {
            let words = codex_add_words(&binary);
            succeeded(agent_status(McpAgent::Codex, &words, &words)?);
        },
        CodexEntry::Current => {},
        CodexEntry::Edited(text) => config.write_config(text)?,
    }
    if let Err(error) = hooks.install(&command) {
        match (&entry, &original) {
            (CodexEntry::Absent, _) => {
                put_back(McpAgent::Codex, &remove_words(McpAgent::Codex, None))
            },
            (CodexEntry::Edited(_), Some(text)) => {
                if let Err(unrestored) = config.write_config(text) {
                    print_error(&CliError::HookFile(unrestored));
                }
            },
            _ => {},
        }
        return Err(error.into());
    }
    drop(deferred);
    hook_added(McpAgent::Codex, &hooks);
    print_note(CODEX_TRUST_NOTE);
    Ok(())
}

/// Says that `agent` now runs the hook from `file`.
fn hook_added(agent: McpAgent, file: &HookFile) {
    print_note(&format!(
        "{} {HOOK_ADDED} {}",
        agent.get_message().unwrap_or_default(),
        file.path().display()
    ));
}

/// Takes muster's hook out of `file` once the server is unregistered,
/// saying so when it was there, and reporting, without failing the
/// command, a file that cannot give it up.
fn unhooked(file: &HookFile) {
    match file.uninstall() {
        Ok(true) => print_note(&format!("{HOOK_REMOVED} {}", file.path().display())),
        Ok(false) => {},
        Err(source) => print_error(&CliError::HookLeft {
            path: file.path().clone(),
            source,
        }),
    }
}

/// Runs `agent`'s CLI with `words` and waits for it, its output shown. A
/// failure to start names `shown`, the command a person runs once the
/// agent is installed.
///
/// # Errors
/// Returns [`CliError::AgentMissing`] or [`CliError::AgentNotRunnable`]
/// when the agent cannot be run.
fn agent_status(
    agent: McpAgent,
    words: &[OsString],
    shown: &[OsString],
) -> Result<ExitStatus, CliError> {
    run(agent, words, shown, AgentOutput::Shown).map(|(status, _)| status)
}

/// Runs `agent`'s CLI with `words` as a step muster takes on its own: in a
/// process group of its own, its output dropped and what it said on stderr
/// kept.
///
/// # Errors
/// Returns what [`agent_status`] returns.
fn hidden_run(
    agent: McpAgent,
    words: &[OsString],
    shown: &[OsString],
) -> Result<(ExitStatus, String), CliError> {
    run(agent, words, shown, AgentOutput::Hidden)
}

/// Runs `agent`'s CLI with `words`, its output shown or hidden as `output`
/// says, and returns how it ended and, hidden, what it said on stderr.
///
/// # Errors
/// Returns what [`agent_status`] returns.
fn run(
    agent: McpAgent,
    words: &[OsString],
    shown: &[OsString],
    output: AgentOutput,
) -> Result<(ExitStatus, String), CliError> {
    let program = program(agent);
    let mut command = Command::new(program);
    command.args(words);
    let ran = match output {
        AgentOutput::Shown => command.status().map(|status| (status, String::new())),
        AgentOutput::Hidden => command
            .process_group(OWN_GROUP)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()
            .map(|done| {
                let said = String::from_utf8_lossy(&done.stderr).trim().to_owned();
                (done.status, said)
            }),
    };
    ran.map_err(|source| run_failure(program, source, shown))
}

/// The failure of starting `program` with `words`: missing, with the
/// command to run once it is installed, or not runnable.
fn run_failure(program: &'static str, source: io::Error, words: &[OsString]) -> CliError {
    if source.kind() != ErrorKind::NotFound {
        return CliError::AgentNotRunnable { program, source };
    }
    CliError::AgentMissing {
        program,
        source,
        command: typed_command(program, words),
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

/// The arguments of Codex's command registering `binary` as muster's
/// server.
fn codex_add_words(binary: &str) -> Vec<OsString> {
    [
        AGENT_MCP,
        ADD_VERB,
        MCP_CONNECTION_KEY,
        END_OF_OPTIONS,
        binary,
        MCP_SUBCOMMAND,
    ]
    .into_iter()
    .map(OsString::from)
    .collect()
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

    /// When the entry muster added cannot be taken out, the way back it
    /// names is the remove and then the add of the old entry, not the add
    /// alone, which would fail as the name is taken.
    #[test]
    fn restoring_a_replaced_entry_names_the_remove_and_the_add() {
        let previous = json!({"type": "stdio", "command": "/old/muster", "args": ["mcp"]});

        let steps = restoring_words(McpScope::User, Some(&previous));

        assert_eq!(
            typed_commands(McpAgent::Claude, &steps),
            format!(
                "claude mcp remove --scope user muster && claude mcp add-json --scope user muster '{previous}'"
            )
        );
        assert_eq!(
            typed_commands(McpAgent::Claude, &restoring_words(McpScope::User, None)),
            "claude mcp remove --scope user muster"
        );
    }

    /// Codex is given the binary as muster's server.
    #[test]
    fn codex_registers_the_binary() {
        assert_eq!(texts(&codex_add_words("/opt/bin/muster")), [
            "mcp",
            "add",
            "muster",
            "--",
            "/opt/bin/muster",
            "mcp"
        ]);
    }
}
