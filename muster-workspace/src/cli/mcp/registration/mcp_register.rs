//! `muster mcp add` and `muster mcp remove`: a passthrough to the agent's
//! own plugin commands, which install muster's plugin, pinned to this
//! release, from the marketplace this repository is, or uninstall it.
//! muster edits no agent config file.

use std::{
    io::ErrorKind,
    iter,
    os::unix::process::ExitStatusExt,
    process::{Command, ExitStatus},
};

use muster_client::{runnable, this_binary};
use muster_domain::constants::{MCP_CONNECTION_KEY, MCP_SUBCOMMAND};
use serde_json::json;
use strum::EnumProperty;

use crate::cli::{
    failure::error::CliError,
    mcp::registration::{
        mcp_agent::{AFTER_ADD, INSTALL_VERB, McpAgent, PROGRAM, REF_MARK, UNINSTALL_VERB},
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
/// This muster's version, whose release tag the marketplace is pinned to.
const VERSION: &str = env!("CARGO_PKG_VERSION");
/// Opens a release's git tag, before its version.
const TAG_PREFIX: &str = "v";
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
/// Returns the code to exit with when an agent's command failed, after which
/// nothing more runs: its own, or for a signal what a shell reports.
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
    let Some(program) = agent.get_str(PROGRAM) else {
        zed(change)?;
        return Ok(None);
    };
    if change == PluginChange::Remove {
        return run(program, &remove_commands(agent), false);
    }
    let failed = run(program, &add_commands(agent, &release_tag()), true)?;
    if failed.is_none()
        && let Some(note) = agent.get_str(AFTER_ADD)
    {
        print_note(note);
    }
    Ok(failed)
}

/// This release's git tag, which the marketplace is pinned to.
fn release_tag() -> String {
    format!("{TAG_PREFIX}{VERSION}")
}

/// The marketplace source `agent` adds: muster's repository pinned to
/// `tag`, in the agent's own form.
fn pinned(agent: McpAgent, tag: &str) -> String {
    let mark = agent.get_str(REF_MARK).unwrap_or_default();
    format!("{MARKETPLACE_REPO}{mark}{tag}")
}

/// The arguments of each command `agent` runs to add the plugin, in order:
/// the marketplace pinned to `tag`, which both agents accept again from the
/// same source, then the plugin from it.
fn add_commands(agent: McpAgent, tag: &str) -> Vec<Vec<String>> {
    let install = agent.get_str(INSTALL_VERB).unwrap_or_default();
    vec![
        vec![
            PLUGIN.to_owned(),
            MARKETPLACE.to_owned(),
            MARKETPLACE_ADD.to_owned(),
            pinned(agent, tag),
        ],
        [PLUGIN, install, PLUGIN_ID]
            .into_iter()
            .map(str::to_owned)
            .collect(),
    ]
}

/// The arguments of the command `agent` runs to uninstall the plugin; the
/// marketplace stays.
fn remove_commands(agent: McpAgent) -> Vec<Vec<String>> {
    let uninstall = agent.get_str(UNINSTALL_VERB).unwrap_or_default();
    vec![
        [PLUGIN, uninstall, PLUGIN_ID]
            .into_iter()
            .map(str::to_owned)
            .collect(),
    ]
}

/// Runs `program` with each of `commands` in turn, its output shown,
/// stopping at the first that fails. Returns the code to exit with for it.
///
/// # Errors
/// Returns [`CliError::AgentMissing`], naming `commands` to run once it is
/// installed, when the agent is not found and `name_when_missing`; else
/// [`CliError::AgentNotRunnable`].
fn run(
    program: &'static str,
    commands: &[Vec<String>],
    name_when_missing: bool,
) -> Result<Option<i32>, CliError> {
    for words in commands {
        let status = Command::new(program)
            .args(words)
            .status()
            .map_err(|source| {
                if name_when_missing && source.kind() == ErrorKind::NotFound {
                    CliError::AgentMissing {
                        program,
                        source,
                        command: typed(program, commands),
                    }
                } else {
                    CliError::AgentNotRunnable { program, source }
                }
            })?;
        if !status.success() {
            return Ok(Some(exit_code(status)));
        }
    }
    Ok(None)
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

    use muster_domain::constants::{APP_NAME, MCP_CONNECTION_KEY, MCP_SUBCOMMAND};
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
    /// The tag the command tests pin to.
    const TAG: &str = "v1.2.3";

    /// The JSON of `path` under the repository root.
    fn manifest(path: &Path) -> Value {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(REPOSITORY)
            .join(path);
        let text = fs::read_to_string(&path).unwrap_or_else(|error| panic!("{path:?}: {error}"));
        serde_json::from_str(&text).unwrap_or_else(|error| panic!("{path:?}: {error}"))
    }

    /// The manifests parse and agree with the code: the marketplace and
    /// plugin names make [`PLUGIN_ID`], the repository is
    /// [`MARKETPLACE_REPO`], the version is this crate's, the server runs
    /// the bare `muster mcp`, and the one SessionStart hook, matching every
    /// start, runs `muster mcp context` so it never fails or prints an
    /// error, within a limit past the context's own deadline.
    #[test]
    fn the_plugin_manifests_name_the_server_and_the_session_start_hook() {
        let marketplace = manifest(Path::new(MARKETPLACE_MANIFEST));
        let (plugin_name, marketplace_name) = PLUGIN_ID.split_once('@').unwrap();
        assert_eq!(marketplace["name"], marketplace_name);
        let [entry] = marketplace["plugins"].as_array().unwrap().as_slice() else {
            panic!("one plugin: {marketplace}");
        };
        assert_eq!(entry["name"], plugin_name);
        assert_eq!(entry["repository"], format!("{GITHUB}{MARKETPLACE_REPO}"));
        let folder = Path::new(entry["source"].as_str().unwrap());

        let plugin = manifest(&folder.join(PLUGIN_MANIFEST));
        assert_eq!(plugin["name"], plugin_name);
        assert_eq!(plugin["version"], VERSION);

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
            format!("{{ {APP_NAME} {MCP_SUBCOMMAND} {CONTEXT_SUBCOMMAND} || true; }} 2>/dev/null"),
            "a hook that never fails or prints an error"
        );
        assert!(
            hook["timeout"].as_u64().unwrap() > CONTEXT_DEADLINE.as_secs(),
            "the context ends well inside the hook's limit: {hook}"
        );
    }

    /// Claude Code adds the marketplace pinned with `#tag`, installs, and
    /// uninstalls.
    #[test]
    fn claude_pins_the_marketplace_and_installs_and_uninstalls_the_plugin() {
        assert_eq!(add_commands(McpAgent::Claude, TAG), [
            vec!["plugin", "marketplace", "add", "sagoez/muster#v1.2.3"],
            vec!["plugin", "install", "muster@muster"],
        ]);
        assert_eq!(remove_commands(McpAgent::Claude), [vec![
            "plugin",
            "uninstall",
            "muster@muster"
        ]]);
    }

    /// Codex adds the marketplace pinned with `@tag`, adds the plugin, and
    /// removes it.
    #[test]
    fn codex_pins_the_marketplace_and_adds_and_removes_the_plugin() {
        assert_eq!(add_commands(McpAgent::Codex, TAG), [
            vec!["plugin", "marketplace", "add", "sagoez/muster@v1.2.3"],
            vec!["plugin", "add", "muster@muster"],
        ]);
        assert_eq!(remove_commands(McpAgent::Codex), [vec![
            "plugin",
            "remove",
            "muster@muster"
        ]]);
    }

    /// The tag is this release's own.
    #[test]
    fn the_marketplace_is_pinned_to_this_release() {
        assert_eq!(release_tag(), format!("v{}", env!("CARGO_PKG_VERSION")));
        assert_eq!(
            pinned(McpAgent::Claude, &release_tag()),
            format!("sagoez/muster#v{}", env!("CARGO_PKG_VERSION"))
        );
    }

    /// An add is named as a person types it, each command in a row.
    #[test]
    fn an_add_is_named_as_a_person_types_it() {
        assert_eq!(
            typed("claude", &add_commands(McpAgent::Claude, TAG)),
            "claude plugin marketplace add 'sagoez/muster#v1.2.3' && claude plugin install muster@muster"
        );
    }
}
