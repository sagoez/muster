//! `muster mcp add` and `muster mcp remove`: the agent's own CLI registering
//! or unregistering muster's MCP server, its output and failure passed
//! through, and muster's session-start hook put into or taken out of the
//! hooks file of the scope registered. An existing registration is never
//! replaced, and a hook never outlives its server.

use std::{
    ffi::OsString,
    io::{self, ErrorKind},
    os::unix::process::ExitStatusExt,
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Stdio, exit},
};

use muster_client::{runnable, this_binary};
use muster_domain::constants::{APP_NAME, MCP_CONNECTION_KEY, MCP_SUBCOMMAND};
use muster_os::sanitize::printable_lines;
use serde_json::{Value, json};
use signal_hook::low_level::emulate_default_handler;
use strum::{EnumMessage, EnumProperty, IntoEnumIterator};

use crate::cli::{
    command_line::invocation::Invocation,
    failure::error::CliError,
    mcp::{
        agent_keys::{ARGS_KEY, COMMAND_KEY, TRANSPORT_KEY},
        claude_config::ClaudeConfig,
        codex_config::CodexConfig,
        git_excludes::{GitExcludes, LOCAL_SETTINGS_RULE},
        hook_command::context_command,
        hook_file::{HookFile, hook_group_text},
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
/// The agents' verb that registers a server.
const ADD_VERB: &str = "add";
/// The agents' verb that unregisters one.
const REMOVE_VERB: &str = "remove";
/// Codex's verb that shows one server.
const GET_VERB: &str = "get";
/// Asks Codex for a server as JSON.
const JSON_FLAG: &str = "--json";
/// Claude Code's flag naming the scope.
const SCOPE_FLAG: &str = "--scope";
/// Ends the agent's own options; the server's command line follows.
const END_OF_OPTIONS: &str = "--";
/// Between the words of a command that cannot be quoted, said only so a
/// person can still read it.
const WORD_SEPARATOR: &str = " ";
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
/// What precedes the file that held no muster hook.
const HOOK_ABSENT: &str = "no muster SessionStart hook to take out of";
/// Follows the agent's name when muster is already registered with it.
const ALREADY: &str = "already has muster registered";
/// Follows the scope, before the registered command.
const RUNNING: &str = "running";
/// Says the registration was left alone and how to change it.
const LEFT_ALONE: &str = "muster left it as it is; to change it, run:";
/// What a scope is introduced by in a message.
const IN_SCOPE: &str = "in the scope";
/// The words of muster's own command, in a hint.
const MUSTER_MCP: &str = "muster mcp";
/// Between two commands a person runs one after the other.
const AND_THEN: &str = "&&";
/// Follows a servers file add could not read: who decides instead.
const UNREAD_ENTRY: &str = "muster could not tell whether muster is registered there, so it asks Claude Code to add it, which never replaces an entry";
/// Follows a servers file remove could not read.
const UNREAD_SKIPPED: &str = "muster only reads it to find its hook, and goes on without it";
/// What precedes a settings file another scope's hook already serves.
const HOOK_SHARED: &str = "muster's SessionStart hook is already in";
/// What precedes a settings file whose hook stays for another scope.
const HOOK_KEPT: &str = "kept muster's SessionStart hook in";
/// What precedes the other scopes reading the same settings file, which
/// hold muster or might.
const SHARED_WITH: &str = "the same file muster's registration also uses in the scope";
/// Between the scopes of a list.
const LIST_SEPARATOR: &str = ", ";
/// Precedes the rule that now keeps local settings out of git.
const IGNORED: &str = "kept .claude/settings.local.json out of git, as Claude Code does, with";
/// Precedes the excludes file the rule went in.
const IGNORED_IN: &str = "in";
/// Precedes the local settings file that could not be kept out of git.
const NOT_IGNORED: &str = "could not keep out of git";
/// Says how to keep it out of git by hand.
const IGNORE_BY_HAND: &str = "add to your global git excludes file the rule";
/// Opens Codex's line saying a server it was asked for does not exist.
const CODEX_NO_SERVER: &str = "No MCP server named";
/// Closes that line.
const CODEX_NOT_FOUND: &str = "found";
/// Opens a warning line of Codex's own, not the failure itself.
const CODEX_WARNING: &str = "WARNING:";
/// Between the lines of what an agent said, in one message.
const DETAIL_SEPARATOR: &str = "; ";

/// `muster mcp add`: when muster is not yet registered with `args`' agent
/// in the scope, registers it through the agent's own CLI; either way puts
/// muster's session-start hook into that scope's hooks file. A registration
/// already there is never touched; it is reported with how to change it.
/// For Zed, prints what to add by hand.
///
/// # Errors
/// Returns [`CliError::Usage`] when `project_given`, as the agent works in
/// the current folder, [`CliError::UserScopeOnly`] for a scope other than
/// `user` with an agent other than Claude Code, [`CliError::BinaryGone`]
/// when this binary was replaced or removed, [`CliError::UnrepresentablePath`]
/// for a binary path that is not UTF-8, [`CliError::AgentMissing`] or
/// [`CliError::AgentNotRunnable`] when the agent cannot be run,
/// [`CliError::CurrentDir`] when the current folder cannot be read,
/// [`CliError::HookFile`] when no home folder holds the agent's config,
/// [`CliError::RegistrationUnknown`] when Codex cannot say whether muster
/// is registered, and [`CliError::HookNotAdded`], naming the hook to add by
/// hand, when the hooks file cannot take it; the registration then stays.
pub fn add(
    invocation: &Invocation<'_>,
    args: &McpRegistration,
    project_given: bool,
) -> Result<(), CliError> {
    match checked(args, project_given)? {
        McpAgent::Zed => {
            let entry = json!({ COMMAND_KEY: own_binary()?, ARGS_KEY: [MCP_SUBCOMMAND] });
            let entry = serde_json::to_string_pretty(&entry).unwrap_or_default();
            emit_line(&format!("\"{MCP_CONNECTION_KEY}\": {entry}"));
            print_note(ZED_ADD_NOTE);
            Ok(())
        },
        McpAgent::Claude => add_claude(
            &ClaudeConfig::find(invocation.cwd()?)?,
            args.scope().unwrap_or(ADD_SCOPE),
        ),
        McpAgent::Codex => add_codex(),
    }
}

/// `muster mcp remove`: takes muster's hook out of the hooks file of the
/// scope the agent removes muster from, then runs the agent's CLI
/// unregistering muster, saying what each step did. A remove the agent
/// refuses ends muster as the agent's CLI ended, its hook already gone, so
/// a hook never outlives its server; a hooks file that cannot give the hook
/// up stops the command before the agent runs. For Zed, says what to delete
/// by hand.
///
/// # Errors
/// Returns what [`add`] returns, but [`CliError::BinaryGone`],
/// [`CliError::RegistrationUnknown`] and [`CliError::HookNotAdded`], and
/// [`CliError::HookFile`] too when the hooks file cannot give the hook up.
pub fn remove(
    invocation: &Invocation<'_>,
    args: &McpRegistration,
    project_given: bool,
) -> Result<(), CliError> {
    match checked(args, project_given)? {
        McpAgent::Zed => {
            print_note(ZED_REMOVE_NOTE);
            Ok(())
        },
        McpAgent::Claude => remove_claude(&ClaudeConfig::find(invocation.cwd()?)?, args.scope()),
        McpAgent::Codex => {
            unhooked(&CodexConfig::find()?.hooks())?;
            let words = remove_words(McpAgent::Codex, None);
            succeeded(agent_status(McpAgent::Codex, &words, &words)?);
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

/// Registers muster with Claude Code in `scope` unless it is already, and
/// puts its hook in that scope's settings file. A project's shared files
/// name the bare `muster`, found on each person's own `PATH`, and its hook
/// runs only where muster is installed. When the scope's servers file
/// cannot be read, Claude Code is asked to add muster, which it refuses
/// over an entry already there. A local settings file muster makes is kept
/// out of git as Claude Code keeps it.
///
/// # Errors
/// Returns what [`add`] returns.
fn add_claude(config: &ClaudeConfig, scope: McpScope) -> Result<(), CliError> {
    let shared = scope == McpScope::Project;
    let binary = if shared {
        APP_NAME.to_owned()
    } else {
        own_binary()?
    };
    let command = context_command(&binary, shared)?;
    let hooks = config.settings(scope);
    let existing = config.entry(scope).unwrap_or_else(|error| {
        print_note(&format!("{error}; {UNREAD_ENTRY}"));
        None
    });
    if existing.is_none() {
        let words = claude_add_words(scope, &binary);
        succeeded(agent_status(McpAgent::Claude, &words, &words)?);
    }
    let made = !hooks.path().exists();
    let users = live_sharers(config, scope);
    if !users.is_empty() && hooks.holds_hook().unwrap_or(false) {
        print_note(&format!(
            "{HOOK_SHARED} {}, {SHARED_WITH} {}",
            hooks.path().display(),
            scope_list(&users)
        ));
    } else {
        hooked(McpAgent::Claude, &hooks, &command)?;
    }
    if made && scope == McpScope::Local && hooks.path().exists() {
        kept_out_of_git(hooks.path());
    }
    if let Some(entry) = existing {
        report_registered(McpAgent::Claude, &entry, Some(scope));
    }
    Ok(())
}

/// Unregisters muster from Claude Code, from `scope` or, without one, from
/// the one scope holding it, taking its hook out of that scope's settings
/// file first, unless another scope muster stays in reads that same file.
/// With `scope`, no servers file is read but one whose scope shares the
/// settings file; without one, a servers file that cannot be read is
/// passed over with a warning, and with muster in more or fewer than one
/// scope Claude Code refuses and no hook is touched. Claude Code runs
/// whatever the servers files hold.
///
/// # Errors
/// Returns what [`remove`] returns.
fn remove_claude(config: &ClaudeConfig, scope: Option<McpScope>) -> Result<(), CliError> {
    let removing = scope.or_else(|| {
        let all: Vec<McpScope> = McpScope::iter().collect();
        let (holding, unread) = config.holding(&all);
        warn_unread(&unread);
        match holding
            .iter()
            .filter(|(_, held)| *held == Some(true))
            .collect::<Vec<_>>()
            .as_slice()
        {
            [(only, _)] => Some(*only),
            _ => None,
        }
    });
    if let Some(removing) = removing {
        let hooks = config.settings(removing);
        let users = live_sharers(config, removing);
        if users.is_empty() {
            unhooked(&hooks)?;
        } else {
            print_note(&format!(
                "{HOOK_KEPT} {}, {SHARED_WITH} {}",
                hooks.path().display(),
                scope_list(&users)
            ));
        }
    }
    let words = remove_words(McpAgent::Claude, scope);
    succeeded(agent_status(McpAgent::Claude, &words, &words)?);
    Ok(())
}

/// The other scopes reading `scope`'s settings file that hold muster, or
/// whose servers file cannot be read and so might, warning of each such
/// file.
fn live_sharers(config: &ClaudeConfig, scope: McpScope) -> Vec<McpScope> {
    let sharing = config.sharing(scope);
    if sharing.is_empty() {
        return sharing;
    }
    let (holding, unread) = config.holding(&sharing);
    warn_unread(&unread);
    holding
        .into_iter()
        .filter(|(_, held)| *held != Some(false))
        .map(|(scope, _)| scope)
        .collect()
}

/// Warns of each servers file in `unread`, which muster passes over.
fn warn_unread(unread: &[HookFileError]) {
    for error in unread {
        print_note(&format!("{error}; {UNREAD_SKIPPED}"));
    }
}

/// `scopes` named in a message.
fn scope_list(scopes: &[McpScope]) -> String {
    scopes
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(LIST_SEPARATOR)
}

/// Keeps the local settings file at `path`, just made, out of git with the
/// rule Claude Code uses, warning when it cannot.
fn kept_out_of_git(path: &Path) {
    let outcome = GitExcludes::find()
        .ok_or(HookFileError::NoHome(McpAgent::Claude))
        .and_then(|excludes| {
            excludes
                .keep_local_settings_out()
                .map(|wrote| (wrote, excludes))
        });
    match outcome {
        Ok((true, excludes)) => print_note(&format!(
            "{IGNORED} {LOCAL_SETTINGS_RULE} {IGNORED_IN} {}",
            excludes.path().display()
        )),
        Ok((false, _)) => {},
        Err(error) => print_note(&format!(
            "{NOT_IGNORED} {}: {error}; {IGNORE_BY_HAND} {LOCAL_SETTINGS_RULE}",
            path.display()
        )),
    }
}

/// Registers muster with Codex unless it is already, and puts its hook in
/// Codex's hooks file.
///
/// # Errors
/// Returns what [`add`] returns.
fn add_codex() -> Result<(), CliError> {
    let binary = own_binary()?;
    let command = context_command(&binary, false)?;
    let hooks = CodexConfig::find()?.hooks();
    let words = codex_add_words(&binary);
    let existing = codex_entry(&words)?;
    if existing.is_none() {
        succeeded(agent_status(McpAgent::Codex, &words, &words)?);
    }
    hooked(McpAgent::Codex, &hooks, &command)?;
    if let Some(transport) = existing {
        report_registered(McpAgent::Codex, &transport, None);
    }
    print_note(CODEX_TRUST_NOTE);
    Ok(())
}

/// The transport of muster's Codex server, as `codex mcp get --json`
/// prints it, when Codex has one; null when it does but prints no JSON;
/// none only when Codex says it has no server named muster. A failure to
/// start names `shown`, the add a person runs once Codex is installed.
///
/// # Errors
/// Returns [`CliError::AgentMissing`] or [`CliError::AgentNotRunnable`]
/// when Codex cannot be run, and [`CliError::RegistrationUnknown`] with
/// what Codex said when it fails any other way, so an entry it could not
/// read is never replaced.
fn codex_entry(shown: &[OsString]) -> Result<Option<Value>, CliError> {
    let program = program(McpAgent::Codex);
    let output = Command::new(program)
        .args([AGENT_MCP, GET_VERB, MCP_CONNECTION_KEY, JSON_FLAG])
        .stdin(Stdio::null())
        .output()
        .map_err(|source| run_failure(program, source, shown))?;
    if output.status.success() {
        return Ok(Some(
            serde_json::from_slice::<Value>(&output.stdout)
                .ok()
                .and_then(|server| server.get(TRANSPORT_KEY).cloned())
                .unwrap_or(Value::Null),
        ));
    }
    let said = String::from_utf8_lossy(&output.stderr);
    if said.contains(&codex_not_found()) {
        return Ok(None);
    }
    let lines: Vec<String> = printable_lines(&said)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with(CODEX_WARNING))
        .map(str::to_owned)
        .collect();
    let detail = if lines.is_empty() {
        output.status.to_string()
    } else {
        lines.join(DETAIL_SEPARATOR)
    };
    Err(CliError::RegistrationUnknown { program, detail })
}

/// What Codex says, among other words, when it has no muster server.
fn codex_not_found() -> String {
    format!("{CODEX_NO_SERVER} '{MCP_CONNECTION_KEY}' {CODEX_NOT_FOUND}")
}

/// Puts muster's hook running `command` into `hooks`, saying that `agent`
/// now runs it.
///
/// # Errors
/// Returns [`CliError::HookNotAdded`], naming the file and the hook to add
/// by hand, when the file cannot take it.
fn hooked(agent: McpAgent, hooks: &HookFile, command: &str) -> Result<(), CliError> {
    hooks
        .install(command)
        .map_err(|source| CliError::HookNotAdded {
            path: hooks.path().clone(),
            hook: hook_group_text(command),
            source,
        })?;
    print_note(&format!(
        "{} {HOOK_ADDED} {}",
        agent.get_message().unwrap_or_default(),
        hooks.path().display()
    ));
    Ok(())
}

/// Says that `agent` already had muster registered, in `scope` when it has
/// scopes, running what `entry` runs, that muster left it alone, and how to
/// change it.
fn report_registered(agent: McpAgent, entry: &Value, scope: Option<McpScope>) {
    let flags = scope.map_or_else(String::new, |scope| format!(" {SCOPE_FLAG} {scope}"));
    let within = scope.map_or_else(String::new, |scope| format!(" {IN_SCOPE} {scope}"));
    let running =
        entry_command(entry).map_or_else(String::new, |command| format!(", {RUNNING} {command}"));
    print_note(&format!(
        "{} {ALREADY}{within}{running}; {LEFT_ALONE} {MUSTER_MCP} {REMOVE_VERB} {agent}{flags} {AND_THEN} {MUSTER_MCP} {ADD_VERB} {agent}{flags}",
        agent.get_message().unwrap_or_default(),
    ));
}

/// The command line `entry` runs, as a person types it: its `command`
/// followed by its `args`.
fn entry_command(entry: &Value) -> Option<String> {
    let command = entry.get(COMMAND_KEY)?.as_str()?;
    let args: Vec<&str> = entry
        .get(ARGS_KEY)
        .and_then(Value::as_array)
        .map(|args| args.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let words = [command].into_iter().chain(args);
    Some(
        shell_join(words.clone())
            .unwrap_or_else(|_| words.collect::<Vec<_>>().join(WORD_SEPARATOR)),
    )
}

/// Takes muster's hook out of `file`, saying whether it was there.
///
/// # Errors
/// Returns [`CliError::HookFile`] when the file cannot give it up; the
/// agent then does not run, so the hook never outlives its server.
fn unhooked(file: &HookFile) -> Result<(), CliError> {
    let lead = if file.uninstall()? {
        HOOK_REMOVED
    } else {
        HOOK_ABSENT
    };
    print_note(&format!("{lead} {}", file.path().display()));
    Ok(())
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
    let program = program(agent);
    Command::new(program)
        .args(words)
        .status()
        .map_err(|source| run_failure(program, source, shown))
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

/// `program` with `words` as a person types it; a word no shell could
/// carry, which no path holds, is shown as it is.
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

/// The arguments of Claude Code's command registering `binary` as muster's
/// server in `scope`.
fn claude_add_words(scope: McpScope, binary: &str) -> Vec<OsString> {
    [
        AGENT_MCP,
        ADD_VERB,
        SCOPE_FLAG,
        &scope.to_string(),
        MCP_CONNECTION_KEY,
        END_OF_OPTIONS,
        binary,
        MCP_SUBCOMMAND,
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

    /// Claude Code and Codex are each given the binary as muster's server,
    /// with one plain add and no other key.
    #[test]
    fn the_agents_register_the_binary_with_a_plain_add() {
        assert_eq!(
            texts(&claude_add_words(McpScope::Local, "/opt/bin/muster")),
            [
                "mcp",
                "add",
                "--scope",
                "local",
                "muster",
                "--",
                "/opt/bin/muster",
                "mcp"
            ]
        );
        assert_eq!(texts(&codex_add_words("/opt/bin/muster")), [
            "mcp",
            "add",
            "muster",
            "--",
            "/opt/bin/muster",
            "mcp"
        ]);
    }

    /// Codex is taken to have no muster server only on the line it prints
    /// for exactly that.
    #[test]
    fn codex_says_it_has_no_muster_server_in_its_own_words() {
        assert_eq!(codex_not_found(), "No MCP server named 'muster' found");
    }

    /// A registered entry is named by the command line it runs.
    #[test]
    fn an_entry_is_named_by_what_it_runs() {
        let entry = json!({"command": "/my tools/muster", "args": ["mcp", "--as", "reviewer"]});

        assert_eq!(
            entry_command(&entry).as_deref(),
            Some("'/my tools/muster' mcp --as reviewer")
        );
        assert_eq!(entry_command(&json!({"url": "http://x"})), None);
    }
}
