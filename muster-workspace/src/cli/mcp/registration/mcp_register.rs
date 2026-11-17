//! `muster mcp add` and `muster mcp remove`: a pure passthrough to the
//! agent's own plugin commands, which install muster's plugin (its MCP
//! server and SessionStart hook) from the marketplace this repository is,
//! or uninstall it. muster edits no agent config file itself.

use std::{
    io::ErrorKind,
    iter,
    os::unix::process::ExitStatusExt,
    process::{Command, ExitStatus, exit},
};

use muster_client::{runnable, this_binary};
use muster_domain::constants::{MCP_CONNECTION_KEY, MCP_SUBCOMMAND};
use serde_json::json;
use strum::EnumProperty;

use crate::cli::{
    failure::error::CliError,
    mcp::{
        mcp_agent::{AFTER_ADD, INSTALL_VERB, McpAgent, PROGRAM, UNINSTALL_VERB},
        plugin_change::PluginChange,
    },
    present::emit::emit_line,
    rows::report::print_note,
};

/// The marketplace muster's plugin is installed from: this repository, on
/// GitHub, whose `.claude-plugin/marketplace.json` lists it.
pub const MARKETPLACE_SOURCE: &str = "sagoez/muster";
/// muster's plugin as the agents name it: the plugin, then the marketplace
/// it comes from, as `.claude-plugin/marketplace.json` names both.
pub const PLUGIN_ID: &str = "muster@muster";
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
/// What to do with the printed entry in Zed, which has no command for it.
const ZED_ADD_NOTE: &str = "Zed has no command for this: paste the entry above as a key inside the \"context_servers\" object of Zed's settings.json; see https://zed.dev/docs/ai/mcp";
/// What to delete in Zed to unregister muster.
const ZED_REMOVE_NOTE: &str = "Zed has no command for this: delete the \"muster\" key from the \"context_servers\" object of Zed's settings.json; see https://zed.dev/docs/ai/mcp";

/// Runs `agent`'s own plugin commands for `change`, one after another with
/// their output passed through, or for Zed prints what to change by hand.
/// When an agent's command fails, muster exits as it did, a signal as a
/// shell reports one, and runs nothing after it.
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
) -> Result<(), CliError> {
    if project_given {
        return Err(CliError::Usage(PROJECT_REFUSED.to_owned()));
    }
    let Some(program) = agent.get_str(PROGRAM) else {
        return zed(change);
    };
    let commands = plugin_commands(agent, change);
    for words in &commands {
        let status = Command::new(program)
            .args(words)
            .status()
            .map_err(|source| {
                if source.kind() == ErrorKind::NotFound {
                    CliError::AgentMissing {
                        program,
                        source,
                        command: typed(program, &commands),
                    }
                } else {
                    CliError::AgentNotRunnable { program, source }
                }
            })?;
        if !status.success() {
            exit(exit_code(status));
        }
    }
    if change == PluginChange::Add
        && let Some(note) = agent.get_str(AFTER_ADD)
    {
        print_note(note);
    }
    Ok(())
}

/// The arguments of each command `agent` runs for `change`, in order: an add
/// adds the marketplace, which does nothing when it is already there, then
/// installs the plugin from it; a remove uninstalls the plugin and keeps the
/// marketplace. Zed has none.
pub fn plugin_commands(agent: McpAgent, change: PluginChange) -> Vec<Vec<&'static str>> {
    let verb = match change {
        PluginChange::Add => INSTALL_VERB,
        PluginChange::Remove => UNINSTALL_VERB,
    };
    let Some(verb) = agent.get_str(verb) else {
        return Vec::new();
    };
    let plugin = vec![PLUGIN, verb, PLUGIN_ID];
    match change {
        PluginChange::Add => vec![
            vec![PLUGIN, MARKETPLACE, MARKETPLACE_ADD, MARKETPLACE_SOURCE],
            plugin,
        ],
        PluginChange::Remove => vec![plugin],
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
fn typed(program: &str, commands: &[Vec<&str>]) -> String {
    commands
        .iter()
        .map(|words| {
            let words: Vec<&str> = iter::once(program).chain(words.iter().copied()).collect();
            shlex::try_join(words.iter().copied()).unwrap_or_else(|_| words.join(WORD_SEPARATOR))
        })
        .collect::<Vec<_>>()
        .join(COMMAND_SEPARATOR)
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use muster_domain::constants::{APP_NAME, MCP_CONNECTION_KEY, MCP_SUBCOMMAND};
    use serde_json::Value;
    use strum::IntoEnumIterator;

    use super::{MARKETPLACE_SOURCE, McpAgent, PLUGIN_ID, plugin_commands, typed};
    use crate::cli::mcp::{
        mcp_command::{CONTEXT_SUBCOMMAND, HOOK_FLAG},
        plugin_change::PluginChange,
        session_context::CONTEXT_DEADLINE,
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

    #[test]
    fn the_plugin_manifests_name_the_server_and_the_session_start_hook() {
        let marketplace = manifest(Path::new(MARKETPLACE_MANIFEST));
        let (plugin_name, marketplace_name) = PLUGIN_ID.split_once('@').unwrap();
        assert_eq!(marketplace["name"], marketplace_name);
        let repository = marketplace["plugins"][0]["repository"].as_str().unwrap();
        assert_eq!(repository, format!("{GITHUB}{MARKETPLACE_SOURCE}"));
        let [entry] = marketplace["plugins"].as_array().unwrap().as_slice() else {
            panic!("one plugin: {marketplace}");
        };
        assert_eq!(entry["name"], plugin_name);
        let folder = Path::new(entry["source"].as_str().unwrap());

        let plugin = manifest(&folder.join(PLUGIN_MANIFEST));
        assert_eq!(plugin["name"], plugin_name);
        assert_eq!(plugin["version"], env!("CARGO_PKG_VERSION"));

        let servers = manifest(&folder.join(PLUGIN_SERVERS));
        assert_eq!(
            servers["mcpServers"][MCP_CONNECTION_KEY],
            serde_json::json!({"command": APP_NAME, "args": [MCP_SUBCOMMAND]}),
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
                "{{ {APP_NAME} {MCP_SUBCOMMAND} {CONTEXT_SUBCOMMAND} --{HOOK_FLAG} || true; }} 2>/dev/null"
            ),
            "a hook that never fails or prints an error"
        );
        assert!(
            hook["timeout"].as_u64().unwrap() > CONTEXT_DEADLINE.as_secs(),
            "the context ends well inside the hook's limit: {hook}"
        );
    }

    #[test]
    fn claude_adds_the_marketplace_then_installs_and_uninstalls_the_plugin() {
        assert_eq!(plugin_commands(McpAgent::Claude, PluginChange::Add), [
            vec!["plugin", "marketplace", "add", "sagoez/muster"],
            vec!["plugin", "install", "muster@muster"],
        ]);
        assert_eq!(plugin_commands(McpAgent::Claude, PluginChange::Remove), [
            vec!["plugin", "uninstall", "muster@muster"]
        ]);
    }

    #[test]
    fn codex_adds_the_marketplace_then_adds_and_removes_the_plugin() {
        assert_eq!(plugin_commands(McpAgent::Codex, PluginChange::Add), [
            vec!["plugin", "marketplace", "add", "sagoez/muster"],
            vec!["plugin", "add", "muster@muster"],
        ]);
        assert_eq!(plugin_commands(McpAgent::Codex, PluginChange::Remove), [
            vec!["plugin", "remove", "muster@muster"]
        ]);
    }

    #[test]
    fn zed_runs_no_command() {
        for change in [PluginChange::Add, PluginChange::Remove] {
            assert!(plugin_commands(McpAgent::Zed, change).is_empty());
        }
        assert!(
            McpAgent::iter()
                .filter(|agent| *agent != McpAgent::Zed)
                .all(|agent| !plugin_commands(agent, PluginChange::Add).is_empty())
        );
    }

    #[test]
    fn an_add_is_named_as_a_person_types_it() {
        assert_eq!(
            typed(
                "claude",
                &plugin_commands(McpAgent::Claude, PluginChange::Add)
            ),
            "claude plugin marketplace add sagoez/muster && claude plugin install muster@muster"
        );
    }
}
