//! `muster mcp add` and `muster mcp remove`: a passthrough to the agent's
//! own plugin commands, which install muster's plugin, at the latest
//! release, from the marketplace this repository is, or uninstall it.
//! muster edits no agent config file.

use std::{
    io::{self, ErrorKind, Write},
    iter,
    os::unix::process::ExitStatusExt,
    process::{Command, ExitStatus},
    slice,
};

use muster_client::{runnable, this_binary};
use muster_domain::constants::{APP_NAME, MCP_CONNECTION_KEY, MCP_SUBCOMMAND};
use serde_json::json;
use signal_hook::low_level::signal_name;

use crate::cli::{
    failure::error::CliError,
    mcp::registration::{
        agent_cli::AgentCli, agent_signals::AgentSignals, mcp_agent::McpAgent,
        plugin_change::PluginChange,
    },
    present::emit::emit_line,
    rows::report::print_note,
};

/// The repository on GitHub that is muster's marketplace, whose
/// `.claude-plugin/marketplace.json` lists the plugin.
pub const MARKETPLACE_REPO: &str = "sagoez/muster";
/// muster's plugin as the agents name it: the plugin, then the marketplace
/// it comes from, as `.claude-plugin/marketplace.json` names both.
pub const PLUGIN_ID: &str = "muster@muster";
/// The marketplace's name, as `.claude-plugin/marketplace.json` gives it.
const MARKETPLACE_NAME: &str = "muster";
/// The branch every release fast-forwards, which the marketplace is pinned
/// to, so one source serves every muster.
pub const RELEASE_BRANCH: &str = "release";
/// The agents' subcommand for plugins.
const PLUGIN: &str = "plugin";
/// The agents' subcommand, under [`PLUGIN`], for marketplaces.
const MARKETPLACE: &str = "marketplace";
/// The agents' verb that adds a marketplace.
const MARKETPLACE_ADD: &str = "add";
/// Joins the commands of one add as a person would type them in a row.
const COMMAND_SEPARATOR: &str = " && ";
/// Between the words of a command shlex cannot quote.
const WORD_SEPARATOR: &str = " ";
/// The exit code a shell gives a command a signal ended, plus the signal.
const SIGNAL_EXIT_BASE: i32 = 128;
/// The exit code for an agent that ended with neither a code nor a signal.
const AGENT_FAILED: i32 = 1;
/// Why `--project` is refused: the plugin is installed for the user.
const PROJECT_REFUSED: &str =
    "`muster mcp add` and `remove` set muster up for every project, so they take no --project";
/// Leads the command that failed, after the agent's own error.
const FAILED_COMMAND: &str = "muster stopped after this command failed:";
/// The agents' verb that removes a marketplace, which muster only names.
const MARKETPLACE_REMOVE: &str = "remove";
/// `muster mcp`'s subcommand that adds the plugin.
const ADD_SUBCOMMAND: &str = "add";
/// Leads the command that clears a marketplace named muster from another
/// source, which muster never removes itself.
const CONFLICT_LEAD: &str = "a marketplace named muster is already added from another source; to use the release branch, run:";
/// Between that command and the add to run after it.
const CONFLICT_THEN: &str = ", then";
/// Follows the agent's name when a signal ended it, before the signal.
const ENDED_BY: &str = "was ended by";
/// Between the signal and the command it ended.
const DURING: &str = "during";
/// Leads the commands a signal kept from running.
const SKIPPED: &str = "; skipped:";
/// Stands for the skipped commands when the one ended was the last.
const NOTHING_SKIPPED: &str = "nothing";
/// What to do with the printed entry in Zed, which has no command for it.
const ZED_ADD_NOTE: &str = "Zed has no command for this: paste the entry above as a key inside the \"context_servers\" object of Zed's settings.json; see https://zed.dev/docs/ai/mcp";
/// What to delete in Zed to unregister muster.
const ZED_REMOVE_NOTE: &str = "Zed has no command for this: delete the \"muster\" key from the \"context_servers\" object of Zed's settings.json; see https://zed.dev/docs/ai/mcp";

/// Runs `agent`'s own plugin commands for `change`, their output passed
/// through, or for Zed prints what to change by hand. An add pins the
/// marketplace to [`RELEASE_BRANCH`], the same source every time, refreshes
/// it, and installs the plugin from it, so a plugin already installed moves
/// to the latest release; nothing is ever removed. When a command fails,
/// nothing more runs and [`run`] says why. Returns the code to exit with
/// then: the command's own, or for a signal what a shell reports.
///
/// # Errors
/// Returns [`CliError::Usage`] when `project_given`, as the plugin is the
/// user's, [`CliError::BinaryGone`] when Zed's entry cannot name this
/// binary, and [`CliError::AgentMissing`] or [`CliError::AgentNotRunnable`]
/// when the agent cannot be run.
pub fn register(
    agent: McpAgent,
    change: PluginChange,
    project_given: bool,
) -> Result<Option<i32>, CliError> {
    if project_given {
        return Err(CliError::Usage(PROJECT_REFUSED.to_owned()));
    }
    let Some(cli) = agent.cli() else {
        zed(change)?;
        return Ok(None);
    };
    let (commands, conflict) = match change {
        PluginChange::Add => (add_commands(cli), cli.conflict()),
        PluginChange::Remove => (remove_commands(cli), None),
    };
    let failed = run(agent, cli, &commands, conflict)?;
    if failed.is_none()
        && change == PluginChange::Add
        && let Some(note) = cli.after_add()
    {
        print_note(note);
    }
    Ok(failed)
}

/// `words` as owned arguments.
fn words(words: &[&str]) -> Vec<String> {
    words.iter().map(|word| (*word).to_owned()).collect()
}

/// The arguments of the commands `cli` runs to add the plugin: the
/// marketplace pinned to [`RELEASE_BRANCH`], a refresh of it, the plugin
/// from it, and, where installing again keeps the old version, an update.
fn add_commands(cli: AgentCli) -> Vec<Vec<String>> {
    let pinned = format!("{MARKETPLACE_REPO}{}{RELEASE_BRANCH}", cli.ref_mark());
    let mut commands = vec![
        words(&[PLUGIN, MARKETPLACE, MARKETPLACE_ADD, &pinned]),
        words(&[PLUGIN, MARKETPLACE, cli.refresh(), MARKETPLACE_NAME]),
        words(&[PLUGIN, cli.install(), PLUGIN_ID]),
    ];
    commands.extend(
        cli.update()
            .map(|update| words(&[PLUGIN, update, PLUGIN_ID])),
    );
    commands
}

/// The arguments of the command `cli` runs to uninstall the plugin; the
/// marketplace stays.
fn remove_commands(cli: AgentCli) -> Vec<Vec<String>> {
    vec![words(&[PLUGIN, cli.uninstall(), PLUGIN_ID])]
}

/// Runs `agent`'s CLI with each of `commands` in turn, its output shown,
/// stopping at the first that fails. `SIGINT` and `SIGQUIT` reach the agent
/// from the terminal while muster waits. A note names the command that
/// failed, or the signal that ended it and the commands it kept from
/// running; when the first command's output says `conflict`, another names
/// the remove to run before adding again. Returns the code to exit with.
///
/// # Errors
/// Returns what [`not_run`] makes of a failure to start the agent, naming
/// every command to run once it is installed.
fn run(
    agent: McpAgent,
    cli: AgentCli,
    commands: &[Vec<String>],
    conflict: Option<&str>,
) -> Result<Option<i32>, CliError> {
    let program = cli.program();
    let _signals = AgentSignals::hold();
    for (index, words) in commands.iter().enumerate() {
        let not_run = |source| not_run(program, source, &typed(program, commands));
        let mut command = Command::new(program);
        command.args(words);
        let (status, said) = match conflict.filter(|_| index == 0) {
            Some(_) => {
                let output = command.output().map_err(not_run)?;
                let _ = io::stdout().write_all(&output.stdout);
                let _ = io::stderr().write_all(&output.stderr);
                let said = [output.stdout, output.stderr].concat();
                (output.status, String::from_utf8_lossy(&said).into_owned())
            },
            None => (command.status().map_err(not_run)?, String::new()),
        };
        if status.success() {
            continue;
        }
        let failed = typed(program, slice::from_ref(words));
        if let Some(signal) = status.signal() {
            let name = signal_name(signal).map_or_else(|| signal.to_string(), str::to_owned);
            let skipped = &commands[index + 1..];
            let skipped = if skipped.is_empty() {
                NOTHING_SKIPPED.to_owned()
            } else {
                typed(program, skipped)
            };
            print_note(&format!(
                "{program} {ENDED_BY} {name} {DURING} {failed}{SKIPPED} {skipped}"
            ));
        } else {
            print_note(&format!("{FAILED_COMMAND} {failed}"));
            if conflict.is_some_and(|conflict| said.contains(conflict)) {
                print_note(&format!(
                    "{CONFLICT_LEAD} {program} {PLUGIN} {MARKETPLACE} {MARKETPLACE_REMOVE} {MARKETPLACE_NAME}{CONFLICT_THEN} {APP_NAME} {MCP_SUBCOMMAND} {ADD_SUBCOMMAND} {agent}"
                ));
            }
        }
        return Ok(Some(exit_code(status)));
    }
    Ok(None)
}

/// Why `program` did not start: missing, naming `missing` to run once it is
/// installed, else not runnable.
fn not_run(program: &'static str, source: io::Error, missing: &str) -> CliError {
    if source.kind() == ErrorKind::NotFound {
        CliError::AgentMissing {
            program,
            source,
            command: missing.to_owned(),
        }
    } else {
        CliError::AgentNotRunnable { program, source }
    }
}

/// Prints Zed's server entry naming this binary for an add, or what to
/// delete for a remove: Zed has no plugin with hooks, so it gets the server
/// alone.
///
/// # Errors
/// Returns [`CliError::BinaryGone`] when the binary was replaced or removed
/// since it started and nothing runnable is left at its path.
fn zed(change: PluginChange) -> Result<(), CliError> {
    if change == PluginChange::Remove {
        print_note(ZED_REMOVE_NOTE);
        return Ok(());
    }
    let binary = this_binary();
    let binary = runnable(&binary).ok_or(CliError::BinaryGone(binary))?;
    let entry = json!({
        "command": binary.display().to_string(),
        "args": [MCP_SUBCOMMAND],
    });
    let entry = serde_json::to_string_pretty(&entry).unwrap_or_default();
    emit_line(&format!("\"{MCP_CONNECTION_KEY}\": {entry}"));
    print_note(ZED_ADD_NOTE);
    Ok(())
}

/// The code muster exits with for an agent that ended by `status`: its own
/// code, or for a signal what a shell reports.
fn exit_code(status: ExitStatus) -> i32 {
    status.code().unwrap_or_else(|| {
        status
            .signal()
            .map_or(AGENT_FAILED, |signal| SIGNAL_EXIT_BASE + signal)
    })
}

/// `program`'s `commands` as a person would type them, in a row.
fn typed(program: &str, commands: &[Vec<String>]) -> String {
    commands
        .iter()
        .map(|words| {
            let words: Vec<&str> = iter::once(program)
                .chain(words.iter().map(String::as_str))
                .collect();
            shlex::try_join(words.iter().copied()).unwrap_or_else(|_| words.join(WORD_SEPARATOR))
        })
        .collect::<Vec<_>>()
        .join(COMMAND_SEPARATOR)
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use muster_domain::constants::{APP_NAME, APP_VERSION, MCP_CONNECTION_KEY, MCP_SUBCOMMAND};
    use serde_json::Value;

    use super::*;
    use crate::cli::mcp::{
        context::session_context::CONTEXT_DEADLINE, mcp_command::CONTEXT_SUBCOMMAND,
    };

    /// The repository root, which is the plugin's marketplace, from this
    /// crate's folder.
    const REPOSITORY: &str = "..";
    /// The marketplace manifest both agents read.
    const MARKETPLACE_MANIFEST: &str = ".claude-plugin/marketplace.json";
    /// The plugin's manifest, under its folder.
    const PLUGIN_MANIFEST: &str = ".claude-plugin/plugin.json";
    /// The plugin's MCP servers file, under its folder.
    const PLUGIN_SERVERS: &str = ".mcp.json";
    /// The plugin's hooks file, under its folder.
    const PLUGIN_HOOKS: &str = "hooks/hooks.json";
    /// The repository this plugin is published from, as GitHub names it.
    const GITHUB: &str = "https://github.com/";

    /// The JSON of `path` under the repository root.
    fn manifest(path: &Path) -> Value {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(REPOSITORY)
            .join(path);
        let text = fs::read_to_string(&path).unwrap_or_else(|error| panic!("{path:?}: {error}"));
        serde_json::from_str(&text).unwrap_or_else(|error| panic!("{path:?}: {error}"))
    }

    /// The manifests parse and agree with the code and with this release:
    /// the marketplace and plugin names make [`PLUGIN_ID`], the repository
    /// is [`MARKETPLACE_REPO`], both versions are this crate's, so the
    /// release branch holds the plugin of the release on it, the server runs the bare `muster
    /// mcp`, and the one SessionStart hook, matching every start, runs
    /// `muster mcp context` so it never fails or prints an error, within a
    /// limit past the context's own deadline. It runs under `sh`, as Codex
    /// hands a hook to the user's login shell, which may not be POSIX.
    #[test]
    fn the_plugin_manifests_name_the_server_and_the_session_start_hook() {
        let marketplace = manifest(Path::new(MARKETPLACE_MANIFEST));
        let (plugin_name, marketplace_name) = PLUGIN_ID.split_once('@').unwrap();
        assert_eq!(marketplace["name"], marketplace_name);
        assert_eq!(marketplace_name, MARKETPLACE_NAME);
        let [entry] = marketplace["plugins"].as_array().unwrap().as_slice() else {
            panic!("one plugin: {marketplace}");
        };
        assert_eq!(entry["name"], plugin_name);
        assert_eq!(entry["version"], APP_VERSION);
        assert_eq!(entry["repository"], format!("{GITHUB}{MARKETPLACE_REPO}"));
        let folder = Path::new(entry["source"].as_str().unwrap());

        let plugin = manifest(&folder.join(PLUGIN_MANIFEST));
        assert_eq!(plugin["name"], plugin_name);
        assert_eq!(plugin["version"], APP_VERSION);

        let servers = manifest(&folder.join(PLUGIN_SERVERS));
        assert_eq!(
            servers["mcpServers"][MCP_CONNECTION_KEY],
            json!({"command": APP_NAME, "args": [MCP_SUBCOMMAND]}),
            "the bare muster from PATH"
        );

        let hooks = manifest(&folder.join(PLUGIN_HOOKS));
        let [group] = hooks["hooks"]["SessionStart"]
            .as_array()
            .unwrap()
            .as_slice()
        else {
            panic!("one SessionStart group: {hooks}");
        };
        assert!(
            group.get("matcher").is_none(),
            "every session start: {group}"
        );
        let [hook] = group["hooks"].as_array().unwrap().as_slice() else {
            panic!("one hook: {group}");
        };
        assert_eq!(hook["type"], "command");
        assert_eq!(
            hook["command"],
            format!(
                "sh -c '{{ {APP_NAME} {MCP_SUBCOMMAND} {CONTEXT_SUBCOMMAND} || true; }} 2>/dev/null'"
            ),
            "a hook that never fails or prints an error"
        );
        assert!(
            hook["timeout"].as_u64().unwrap() > CONTEXT_DEADLINE.as_secs(),
            "the context ends well inside the hook's limit: {hook}"
        );
    }

    /// Claude Code adds the marketplace pinned with `#release`, refreshes it,
    /// installs and updates the plugin, and uninstalls it; nothing removes
    /// the marketplace.
    #[test]
    fn claude_pins_the_release_branch_and_updates_and_uninstalls_the_plugin() {
        let cli = McpAgent::Claude.cli().unwrap();
        assert_eq!(add_commands(cli), [
            vec!["plugin", "marketplace", "add", "sagoez/muster#release"],
            vec!["plugin", "marketplace", "update", "muster"],
            vec!["plugin", "install", "muster@muster"],
            vec!["plugin", "update", "muster@muster"],
        ]);
        assert_eq!(remove_commands(cli), [vec![
            "plugin",
            "uninstall",
            "muster@muster"
        ]]);
    }

    /// Codex adds the marketplace pinned with `@release`, upgrades it, adds
    /// the plugin, which moves it to the marketplace's version, and removes
    /// it.
    #[test]
    fn codex_pins_the_release_branch_and_adds_and_removes_the_plugin() {
        let cli = McpAgent::Codex.cli().unwrap();
        assert_eq!(add_commands(cli), [
            vec!["plugin", "marketplace", "add", "sagoez/muster@release"],
            vec!["plugin", "marketplace", "upgrade", "muster"],
            vec!["plugin", "add", "muster@muster"],
        ]);
        assert_eq!(remove_commands(cli), [vec![
            "plugin",
            "remove",
            "muster@muster"
        ]]);
    }

    /// An add is named as a person types it, each command in a row.
    #[test]
    fn an_add_is_named_as_a_person_types_it() {
        assert_eq!(
            typed("codex", &add_commands(McpAgent::Codex.cli().unwrap())),
            "codex plugin marketplace add sagoez/muster@release && codex plugin marketplace upgrade muster && codex plugin add muster@muster"
        );
    }
}
