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
use muster_os::{path::resolved, sanitize::printable_lines};
use serde_json::{Value, json};
use signal_hook::low_level::emulate_default_handler;
use strum::{EnumMessage, EnumProperty, IntoEnumIterator};

use crate::cli::{
    command_line::invocation::Invocation,
    failure::error::CliError,
    mcp::{
        agent_child::{AgentRun, run_forwarding},
        agent_keys::{ARGS_KEY, COMMAND_KEY, NAME_KEY, TRANSPORT_KEY},
        claude_config::ClaudeConfig,
        codex_config::CodexConfig,
        entry_state::EntryState,
        git_excludes::{GitExcludes, LOCAL_SETTINGS_RULE},
        hook_command::{context_command, hook_binary, is_muster_binary},
        hook_file::{HookFile, hook_group_text},
        hook_install::HookInstall,
        mcp_agent::{McpAgent, PROGRAM},
        mcp_registration::McpRegistration,
        mcp_scope::McpScope,
        server_entries::ServerEntries,
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
/// Codex's verb that lists its servers.
const LIST_VERB: &str = "list";
/// Asks Codex for its servers as JSON.
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
/// Precedes a hooks file already holding a muster hook.
const HOOK_PRESENT: &str = "muster's SessionStart hook is already in";
/// Says the hook already there was not changed.
const HOOK_LEFT: &str = "muster left it as it is";
/// Precedes a hooks file whose muster hook was repaired.
const HOOK_REPAIRED: &str = "repaired muster's SessionStart hook in";
/// Precedes the binary a repaired hook ran.
const HOOK_RAN: &str = "it ran";
/// Follows a binary that is gone, before the one the hook now runs.
const GONE_BINARY: &str = ", which no longer exists; it now runs";
/// Precedes the binary a kept registration runs.
const KEPT_RUNS: &str = "the registration muster kept runs";
/// Precedes the binary this muster is.
const NOT_THIS: &str = ", not this muster,";
/// Says the hook follows the kept registration.
const HOOK_FOLLOWS: &str = "; its hook runs the same muster as the registration";
/// Says no registration was found to remove.
const NO_REGISTRATION: &str = "Claude Code holds no muster registration in any scope muster could read, so it did not run; muster's hooks in those scopes' settings files are out";
/// Follows a servers file that could not be read when no scope holds muster.
const UNREAD_UNCHECKED: &str =
    "muster could not tell whether that scope holds muster, so it left that scope's hook";
/// Follows a servers file of a scope sharing the settings file that could
/// not be read.
const UNREAD_KEPT: &str =
    "muster cannot tell whether that scope still uses its hook, so it keeps the hook";
/// Follows a hooks file a remove cannot parse.
const UNPARSABLE_SKIPPED: &str =
    "muster can find no hook of its own in it, so it goes on to the agent's remove";
/// What precedes a settings file whose hook stays for another scope.
const HOOK_KEPT: &str = "kept muster's SessionStart hook in";
/// What precedes the other scopes reading the same settings file, which
/// hold muster or might.
const SHARED_WITH: &str = "the same file muster's registration also uses in the scope";
/// Precedes the servers files that could not be read.
const UNREAD_LEAD: &str = "unknown, as";
/// Between the scopes of a list.
const LIST_SEPARATOR: &str = ", ";
/// Precedes the rule that now keeps local settings out of git.
const IGNORED: &str = "kept .claude/settings.local.json out of git, as Claude Code does, with";
/// Precedes the repository's exclude file the rule went in.
const IGNORED_IN: &str = "in";
/// Precedes the local settings file that could not be kept out of git.
const NOT_IGNORED: &str = "keep this file out of git yourself";
/// Why muster could not keep it out of git.
const NO_REPOSITORY: &str = "the folder is in no git repository, or git is not on PATH";
/// Says how to keep it out of git by hand.
const IGNORE_BY_HAND: &str = "add to the repository's .git/info/exclude the rule";
/// Precedes the agent running when muster was asked to stop.
const STOPPED_LEAD: &str = "muster was asked to stop while";
/// Precedes how the agent then ended.
const STOPPED_RAN: &str = "ran, which ended with";
/// Says muster went no further.
const STOPPED_TAIL: &str = "muster stopped before its next step";
/// Says Codex printed no list of servers, before how it ended.
const NO_LIST: &str = "codex mcp list --json printed no list of servers and ended with";
/// Precedes the add a person runs by hand once Codex reads its config.
const BY_HAND: &str = "once that is fixed, register muster by hand with";
/// Between the commands a person may choose from.
const OR_ELSE: &str = ", or ";
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
/// unregistering muster from that same scope, saying what each step did. A
/// remove the agent refuses ends muster as the agent's CLI ended, its hook
/// already gone, so a hook never outlives its server. A hooks file muster
/// cannot parse holds no hook it could take out: muster warns and still
/// runs the agent's remove; one it cannot read or write stops the command
/// before the agent runs. For Zed, says what to delete by hand.
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
            agent_status(McpAgent::Codex, &remove_words(McpAgent::Codex, None))?;
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
/// makes sure muster's hook is in that scope's settings file, leaving one
/// already there as it is, as another scope sharing the file put it or the
/// user shaped it. A project's shared files name the bare `muster`, found
/// on each person's own `PATH`; every hook ends quietly where muster is
/// missing or too old. When the scope's servers file cannot be read, Claude Code is
/// asked to add muster, which it refuses over an entry already there. A
/// local settings file muster makes is kept out of git as Claude Code keeps
/// it.
///
/// # Errors
/// Returns what [`add`] returns.
fn add_claude(config: &ClaudeConfig, scope: McpScope) -> Result<(), CliError> {
    let shared = scope == McpScope::Project;
    let own = if shared {
        APP_NAME.to_owned()
    } else {
        own_binary()?
    };
    let hooks = config.settings(scope);
    let mut entries = ServerEntries::new(config);
    let existing = match entries.entry(scope) {
        EntryState::Held(entry) => Some(entry),
        EntryState::Absent => None,
        EntryState::Unknown => {
            return Err(entries.take_unread().into_iter().next().map_or(
                CliError::RegistrationUnknown {
                    program: program(McpAgent::Claude),
                    detail: config.servers_path(scope).display().to_string(),
                },
                |source| CliError::ServersUnreadable { source },
            ));
        },
    };
    if existing.is_none() {
        agent_status(McpAgent::Claude, &add_words(Some(scope), &own))?;
    }
    let binary = hook_target(existing.as_ref(), &own);
    let made = !hooks.path().exists();
    hooked(McpAgent::Claude, &hooks, &context_command(&binary)?)?;
    if made && scope == McpScope::Local && hooks.path().exists() {
        kept_out_of_git(config.cwd(), hooks.path());
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
/// settings file. Without one, every scope's servers file is read, and
/// unless exactly one scope is known to hold muster and none is unknown,
/// muster refuses before Claude Code runs and asks for `--scope`, so no
/// hook outlives its server.
///
/// # Errors
/// Returns what [`remove`] returns, and [`CliError::ScopeUnclear`] when the
/// scope to remove from cannot be told.
fn remove_claude(config: &ClaudeConfig, scope: Option<McpScope>) -> Result<(), CliError> {
    let mut entries = ServerEntries::new(config);
    let removing = match scope {
        Some(scope) => scope,
        None => match only_scope(&mut entries)? {
            Some(scope) => scope,
            None => {
                unhooked_nowhere(config, &mut entries);
                return Ok(());
            },
        },
    };
    unhooked_unless_shared(config, &mut entries, removing)?;
    agent_status(
        McpAgent::Claude,
        &remove_words(McpAgent::Claude, Some(removing)),
    )?;
    Ok(())
}

/// Takes muster's hook out of `scope`'s settings file, unless another scope
/// muster stays in reads that same file, saying which.
///
/// # Errors
/// Returns what [`unhooked`] returns.
fn unhooked_unless_shared(
    config: &ClaudeConfig,
    entries: &mut ServerEntries<'_>,
    scope: McpScope,
) -> Result<(), CliError> {
    let hooks = config.settings(scope);
    let users = live_sharers(config, entries, scope);
    if users.is_empty() {
        return unhooked(&hooks);
    }
    print_note(&format!(
        "{HOOK_KEPT} {}, {SHARED_WITH} {}",
        hooks.path().display(),
        scope_list(&users)
    ));
    Ok(())
}

/// With muster registered in no scope muster could read, takes its hooks
/// out of the settings files of the scopes it could read, each file once,
/// and says no registration was found; Claude Code does not run, having
/// nothing of muster's to remove. A hooks file that cannot give its hook
/// up is told of and passed over.
fn unhooked_nowhere(config: &ClaudeConfig, entries: &mut ServerEntries<'_>) {
    let mut seen = Vec::new();
    for scope in McpScope::iter() {
        if entries.entry(scope) == EntryState::Unknown {
            continue;
        }
        let file = resolved(config.settings(scope).path());
        if seen.contains(&file) {
            continue;
        }
        seen.push(file);
        if let Err(error) = unhooked_unless_shared(config, entries, scope) {
            print_note(&error.to_string());
        }
    }
    print_note(NO_REGISTRATION);
}

/// The one scope known to hold muster, every servers file read; none when
/// no scope muster could read holds it, each file it could not read told
/// of.
///
/// # Errors
/// Returns [`CliError::ScopeUnclear`], naming the scopes found to hold
/// muster and the files that could not be read, when more than one scope
/// holds it, or one does while another cannot be read.
fn only_scope(entries: &mut ServerEntries<'_>) -> Result<Option<McpScope>, CliError> {
    let mut holding = Vec::new();
    let mut unknown = Vec::new();
    for scope in McpScope::iter() {
        match entries.entry(scope) {
            EntryState::Held(_) => holding.push(scope),
            EntryState::Absent => {},
            EntryState::Unknown => unknown.push(scope),
        }
    }
    if holding.is_empty() {
        for error in entries.take_unread() {
            print_note(&format!("{error}; {UNREAD_UNCHECKED}"));
        }
        return Ok(None);
    }
    let unread: Vec<String> = entries
        .take_unread()
        .iter()
        .map(ToString::to_string)
        .collect();
    if let [only] = holding.as_slice()
        && unknown.is_empty()
    {
        return Ok(Some(*only));
    }
    Err(CliError::ScopeUnclear {
        found: scope_list(&holding),
        unread: if unread.is_empty() {
            String::new()
        } else {
            format!(
                "{DETAIL_SEPARATOR}{UNREAD_LEAD} {}",
                unread.join(DETAIL_SEPARATOR)
            )
        },
        commands: holding
            .iter()
            .chain(&unknown)
            .map(|scope| {
                format!(
                    "{MUSTER_MCP} {REMOVE_VERB} {} {SCOPE_FLAG} {scope}",
                    McpAgent::Claude
                )
            })
            .collect::<Vec<_>>()
            .join(OR_ELSE),
    })
}

/// The other scopes reading `scope`'s settings file that hold muster, or
/// whose servers file cannot be read and so might, warning of each such
/// file.
fn live_sharers(
    config: &ClaudeConfig,
    entries: &mut ServerEntries<'_>,
    scope: McpScope,
) -> Vec<McpScope> {
    let users: Vec<McpScope> = config
        .sharing(scope)
        .into_iter()
        .filter(|other| entries.entry(*other) != EntryState::Absent)
        .collect();
    for error in entries.take_unread() {
        print_note(&format!("{error}; {UNREAD_KEPT}"));
    }
    users
}

/// `scopes` named in a message.
fn scope_list(scopes: &[McpScope]) -> String {
    scopes
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(LIST_SEPARATOR)
}

/// Keeps the local settings file at `path`, just made in `folder`, out of
/// git with the rule Claude Code uses, in the exclude file of the
/// repository `folder` is in; warns, naming the file, when `folder` is in
/// no repository, git cannot run or the rule cannot be written.
fn kept_out_of_git(folder: &Path, path: &Path) {
    let Some(excludes) = GitExcludes::of(folder) else {
        print_note(&format!(
            "{NOT_IGNORED}: {}; {NO_REPOSITORY}",
            path.display()
        ));
        return;
    };
    match excludes.keep_local_settings_out() {
        Ok(true) => print_note(&format!(
            "{IGNORED} {LOCAL_SETTINGS_RULE} {IGNORED_IN} {}",
            excludes.path().display()
        )),
        Ok(false) => {},
        Err(error) => print_note(&format!(
            "{NOT_IGNORED}: {}; {error}; {IGNORE_BY_HAND} {LOCAL_SETTINGS_RULE}",
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
    let own = own_binary()?;
    let hooks = CodexConfig::find()?.hooks();
    let words = add_words(None, &own);
    let existing = codex_entry(&words)?;
    if existing.is_none() {
        agent_status(McpAgent::Codex, &words)?;
    }
    let binary = hook_target(existing.as_ref(), &own);
    let outcome = hooked(McpAgent::Codex, &hooks, &context_command(&binary)?)?;
    if let Some(transport) = existing {
        report_registered(McpAgent::Codex, &transport, None);
    }
    if outcome.changed() {
        print_note(CODEX_TRUST_NOTE);
    }
    Ok(())
}

/// The transport of muster's Codex server when Codex has one, read from the
/// JSON list of servers `codex mcp list --json` prints; none when the list
/// holds no server named muster. A failure to start names `shown`, the add
/// a person runs once Codex is installed.
///
/// # Errors
/// Returns [`CliError::AgentMissing`] or [`CliError::AgentNotRunnable`]
/// when Codex cannot be run, and [`CliError::RegistrationUnknown`], with
/// what Codex said and the add to run by hand, when it fails or prints no
/// list, so an entry it could not read is never replaced.
fn codex_entry(shown: &[OsString]) -> Result<Option<Value>, CliError> {
    let program = program(McpAgent::Codex);
    let run = run_forwarding(
        Command::new(program)
            .args([AGENT_MCP, LIST_VERB, JSON_FLAG])
            .stdin(Stdio::null()),
        true,
    )
    .map_err(|source| run_failure(program, source, shown))?;
    stop_if_signalled(program, &run);
    if run.status().success()
        && let Ok(Value::Array(servers)) = serde_json::from_slice::<Value>(run.stdout())
    {
        return Ok(codex_server(&servers));
    }
    let lines: Vec<String> = printable_lines(&String::from_utf8_lossy(run.stderr()))
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with(CODEX_WARNING))
        .map(str::to_owned)
        .collect();
    let said = if lines.is_empty() {
        format!("{NO_LIST} {}", run.status())
    } else {
        lines.join(DETAIL_SEPARATOR)
    };
    Err(CliError::RegistrationUnknown {
        program,
        detail: format!(
            "{said}{DETAIL_SEPARATOR}{BY_HAND} {}",
            typed_command(program, shown)
        ),
    })
}

/// The transport of the server named muster among `servers`, as Codex
/// lists them; null when it lists one with no transport.
fn codex_server(servers: &[Value]) -> Option<Value> {
    servers
        .iter()
        .find(|server| server.get(NAME_KEY).and_then(Value::as_str) == Some(MCP_CONNECTION_KEY))
        .map(|server| server.get(TRANSPORT_KEY).cloned().unwrap_or(Value::Null))
}

/// Puts muster's hook running `command` into `hooks`, saying exactly what
/// the one edit did: that `agent` now runs it, that a hook whose binary was
/// gone now runs `command`, or that a muster hook was already there and
/// left as it is. Returns what it did.
///
/// # Errors
/// Returns [`CliError::HookNotAdded`], naming the file and the hook to add
/// by hand, when the file cannot take it.
fn hooked(agent: McpAgent, hooks: &HookFile, command: &str) -> Result<HookInstall, CliError> {
    let outcome = hooks
        .install(command)
        .map_err(|source| CliError::HookNotAdded {
            path: hooks.path().clone(),
            hook: hook_group_text(command),
            source,
        })?;
    match &outcome {
        HookInstall::Added => print_note(&format!(
            "{} {HOOK_ADDED} {}",
            agent.get_message().unwrap_or_default(),
            hooks.path().display()
        )),
        HookInstall::Repaired(gone) => print_note(&format!(
            "{HOOK_REPAIRED} {}{DETAIL_SEPARATOR}{HOOK_RAN} {}{GONE_BINARY} {}",
            hooks.path().display(),
            gone.join(LIST_SEPARATOR),
            hook_binary(command).unwrap_or_default()
        )),
        HookInstall::Present => print_note(&format!(
            "{HOOK_PRESENT} {}{DETAIL_SEPARATOR}{HOOK_LEFT}",
            hooks.path().display()
        )),
    }
    Ok(outcome)
}

/// The binary the hook runs: the one a kept registration `existing` runs,
/// so server and hook are one muster, when muster would know that hook as
/// its own; else `own`, the binary this add registers. Says so in one line
/// when the kept registration runs another muster than `own`.
fn hook_target(existing: Option<&Value>, own: &str) -> String {
    let kept = existing
        .and_then(|entry| entry.get(COMMAND_KEY))
        .and_then(Value::as_str)
        .filter(|kept| is_muster_binary(kept));
    match kept {
        Some(kept) if kept != own => {
            print_note(&format!("{KEPT_RUNS} {kept}{NOT_THIS} {own}{HOOK_FOLLOWS}"));
            kept.to_owned()
        },
        _ => own.to_owned(),
    }
}

/// Says that `agent` already had muster registered, in `scope` when it has
/// scopes, running what `entry` runs, that muster left it alone, and how to
/// change it.
fn report_registered(agent: McpAgent, entry: &Value, scope: Option<McpScope>) {
    let within = scope.map_or_else(String::new, |scope| format!(" {IN_SCOPE} {scope}"));
    let running =
        entry_command(entry).map_or_else(String::new, |command| format!(", {RUNNING} {command}"));
    print_note(&format!(
        "{} {ALREADY}{within}{running}; {LEFT_ALONE} {}",
        agent.get_message().unwrap_or_default(),
        change_hint(agent, scope)
    ));
}

/// The commands that take muster out of `agent` in `scope` and put it back,
/// as a person runs them.
fn change_hint(agent: McpAgent, scope: Option<McpScope>) -> String {
    let flags = scope.map_or_else(String::new, |scope| format!(" {SCOPE_FLAG} {scope}"));
    format!(
        "{MUSTER_MCP} {REMOVE_VERB} {agent}{flags} {AND_THEN} {MUSTER_MCP} {ADD_VERB} {agent}{flags}"
    )
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
    let words: Vec<&str> = [command].into_iter().chain(args).collect();
    Some(typed(&words))
}

/// Takes muster's hook out of `file`, saying whether it was there. A file
/// muster cannot parse holds no hook of muster's it could find, so it is
/// warned of and passed over.
///
/// # Errors
/// Returns [`CliError::HookFile`] when the file cannot be read or written;
/// the agent then does not run, so the hook never outlives its server.
fn unhooked(file: &HookFile) -> Result<(), CliError> {
    let lead = match file.uninstall() {
        Ok(true) => HOOK_REMOVED,
        Ok(false) => HOOK_ABSENT,
        Err(error) if error.is_unparsable() => {
            print_note(&format!("{error}; {UNPARSABLE_SKIPPED}"));
            return Ok(());
        },
        Err(error) => return Err(error.into()),
    };
    print_note(&format!("{lead} {}", file.path().display()));
    Ok(())
}

/// Runs `agent`'s CLI with `words` and waits for it, its output shown,
/// ending muster as the agent ended when it failed. A failure to start
/// names the command a person runs once the agent is installed.
///
/// # Errors
/// Returns [`CliError::AgentMissing`] or [`CliError::AgentNotRunnable`]
/// when the agent cannot be run.
fn agent_status(agent: McpAgent, words: &[OsString]) -> Result<(), CliError> {
    let program = program(agent);
    let run = run_forwarding(Command::new(program).args(words), false)
        .map_err(|source| run_failure(program, source, words))?;
    stop_if_signalled(program, &run);
    succeeded(*run.status());
    Ok(())
}

/// Ends muster when it was asked to stop while `program` ran: says how the
/// agent ended, then ends as the agent did when it failed, else by the
/// signal, never going on to the next step.
fn stop_if_signalled(program: &str, run: &AgentRun) {
    let Some(signal) = *run.stop() else {
        return;
    };
    print_note(&format!(
        "{STOPPED_LEAD} {program} {STOPPED_RAN} {}; {STOPPED_TAIL}",
        run.status()
    ));
    succeeded(*run.status());
    let _ = emulate_default_handler(signal);
    exit(AGENT_FAILED)
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

/// `program` with `words` as a person types it.
fn typed_command(program: &str, words: &[OsString]) -> String {
    let words: Vec<String> = words
        .iter()
        .map(|word| word.to_string_lossy().into_owned())
        .collect();
    let all: Vec<&str> = [program]
        .into_iter()
        .chain(words.iter().map(String::as_str))
        .collect();
    typed(&all)
}

/// `words` joined as a person types them, quoted for a shell; a word no
/// shell could carry, which no path holds, is shown as it is.
fn typed(words: &[&str]) -> String {
    shell_join(words.iter().copied()).unwrap_or_else(|_| words.join(WORD_SEPARATOR))
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

/// The arguments of an agent's command registering `binary` as muster's
/// server, in `scope` for Claude Code; Codex's CLI has no scopes.
fn add_words(scope: Option<McpScope>, binary: &str) -> Vec<OsString> {
    let mut words: Vec<OsString> = vec![AGENT_MCP.into(), ADD_VERB.into()];
    if let Some(scope) = scope {
        words.extend([SCOPE_FLAG.into(), scope.to_string().into()]);
    }
    words.extend(
        [MCP_CONNECTION_KEY, END_OF_OPTIONS, binary, MCP_SUBCOMMAND]
            .into_iter()
            .map(OsString::from),
    );
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
            texts(&add_words(Some(McpScope::Local), "/opt/bin/muster")),
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
        assert_eq!(texts(&add_words(None, "/opt/bin/muster")), [
            "mcp",
            "add",
            "muster",
            "--",
            "/opt/bin/muster",
            "mcp"
        ]);
    }

    /// Codex's muster server is found in its JSON list by name, with its
    /// transport, and only there.
    #[test]
    fn codex_lists_its_muster_server_by_name() {
        let transport = json!({"type": "stdio", "command": "/opt/muster", "args": ["mcp"]});
        let listed = [
            json!({"name": "other", "transport": {"type": "stdio", "command": "x"}}),
            json!({"name": "muster", "enabled": true, "transport": transport}),
        ];

        assert_eq!(codex_server(&listed), Some(transport));
        assert_eq!(codex_server(&listed[..1]), None);
        assert_eq!(
            codex_server(&[json!({"name": "muster"})]),
            Some(Value::Null)
        );
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
