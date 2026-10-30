use std::path::PathBuf;

use strum::IntoEnumIterator;

use super::{
    agent::Agent,
    agent_config_error::AgentConfigError,
    config_locations::ConfigLocations,
    config_target::ConfigTarget,
    edit_outcome::EditOutcome,
    entry_change::EntryChange,
    json_document::{JsonDialect, JsonPlace, edit_json},
    mcp_registry::McpRegistry,
    mcp_scope::McpScope,
    planned_edit::{PlannedEdit, followed, read_text},
    server_entry::ServerEntry,
    server_launch::ServerLaunch,
    server_name::McpServerName,
    toml_document::edit_toml,
};

/// The key Claude Code keeps its servers under, at each level.
const CLAUDE_SERVERS: &str = "mcpServers";
/// The key of Claude Code's per-project settings in `.claude.json`.
const CLAUDE_PROJECTS: &str = "projects";
/// The file of a project's shared Claude Code servers.
const CLAUDE_PROJECT_FILE: &str = ".mcp.json";
/// The key Zed keeps its MCP servers under.
const ZED_SERVERS: &str = "context_servers";
/// Codex's config file, in its home or a project's `.codex` folder.
const CODEX_CONFIG: &str = "config.toml";
/// A project's Codex folder.
const CODEX_PROJECT_DIR: &str = ".codex";
/// What a person should know about a project's `.mcp.json`.
const CLAUDE_PROJECT_NOTE: &str =
    "Claude Code asks before it uses a server from a project's .mcp.json";
/// What a person should know about a project's Codex config.
const CODEX_PROJECT_NOTE: &str =
    "Codex reads a project's .codex/config.toml only once the project is trusted";

/// How an agent's config file is written.
enum Format {
    /// JSON, the table of servers under these keys.
    Json(Vec<String>, JsonDialect),
    /// Codex's TOML.
    Toml,
}

/// Where one config lives and how it is edited.
struct Place {
    /// The file.
    path: PathBuf,
    /// How it is written.
    format: Format,
    /// Whether its folder is made when missing.
    makes_folder: bool,
}

/// The agent configs on disk, at the locations the composition root found
/// for this user, adding entries that launch muster as `launch` says.
#[derive(Clone, Debug)]
pub struct AgentConfigFiles {
    /// Where the user's configs are, or none without a home folder.
    locations: Option<ConfigLocations>,
    /// How an agent starts muster's server.
    launch: ServerLaunch,
}

impl AgentConfigFiles {
    /// The configs at `locations`, launching muster as `launch` says.
    pub fn new(locations: Option<ConfigLocations>, launch: ServerLaunch) -> Self {
        Self { locations, launch }
    }

    /// Where `target`'s config lives and how it is edited.
    ///
    /// # Errors
    /// Returns [`AgentConfigError::NoProject`] for a project's config with
    /// no project, and [`AgentConfigError::NoHome`] for a user's with no
    /// home folder.
    fn place(&self, target: &ConfigTarget) -> Result<Place, AgentConfigError> {
        let agent = target.agent();
        let scope = target.scope();
        let root = || {
            target
                .root()
                .as_ref()
                .map(|root| root.as_ref().clone())
                .ok_or(AgentConfigError::NoProject { scope })
        };
        let home = || {
            self.locations
                .as_ref()
                .ok_or(AgentConfigError::NoHome(agent))
        };
        let place = |path, format, makes_folder| Place {
            path,
            format,
            makes_folder,
        };
        let json = |keys: &[&str], dialect| {
            Format::Json(keys.iter().map(ToString::to_string).collect(), dialect)
        };
        Ok(match (agent, scope) {
            (Agent::Claude, McpScope::Local) => {
                let project = root()?.display().to_string();
                place(
                    home()?.claude_json().clone(),
                    json(
                        &[CLAUDE_PROJECTS, &project, CLAUDE_SERVERS],
                        JsonDialect::Strict,
                    ),
                    false,
                )
            },
            (Agent::Claude, McpScope::Project) => place(
                root()?.join(CLAUDE_PROJECT_FILE),
                json(&[CLAUDE_SERVERS], JsonDialect::Strict),
                false,
            ),
            (Agent::Claude, McpScope::User) => place(
                home()?.claude_json().clone(),
                json(&[CLAUDE_SERVERS], JsonDialect::Strict),
                false,
            ),
            (Agent::Codex, McpScope::User) => {
                place(home()?.codex_home().join(CODEX_CONFIG), Format::Toml, true)
            },
            (Agent::Codex, McpScope::Project) => place(
                root()?.join(CODEX_PROJECT_DIR).join(CODEX_CONFIG),
                Format::Toml,
                true,
            ),
            (Agent::Zed, McpScope::User) => place(
                home()?.zed_settings().clone(),
                json(&[ZED_SERVERS], JsonDialect::WithComments),
                true,
            ),
            (Agent::Codex | Agent::Zed, _) => {
                return Err(AgentConfigError::UnsupportedScope { agent, scope });
            },
        })
    }

    /// Whether `agent`'s config or binary is found.
    fn is_present(&self, agent: Agent) -> bool {
        let Some(locations) = &self.locations else {
            return false;
        };
        let configs: &[&PathBuf] = match agent {
            Agent::Claude => &[locations.claude_json(), locations.claude_dir()],
            Agent::Codex => &[locations.codex_home()],
            Agent::Zed => &[locations.zed_settings()],
        };
        configs.iter().any(|path| path.exists()) || locations.on_path(agent.binaries())
    }
}

impl McpRegistry for AgentConfigFiles {
    fn present(&self) -> Vec<Agent> {
        Agent::iter()
            .filter(|agent| self.is_present(*agent))
            .collect()
    }

    fn plan(
        &self,
        target: &ConfigTarget,
        name: &McpServerName,
        change: EntryChange,
        force: bool,
    ) -> Result<PlannedEdit, AgentConfigError> {
        let agent = target.agent();
        let scope = target.scope();
        if !agent.supports(scope) {
            return Err(AgentConfigError::UnsupportedScope { agent, scope });
        }
        let place = self.place(target)?;
        let entry = ServerEntry::builder()
            .command(self.launch.command_text()?.to_owned())
            .args(self.launch.args().clone())
            .with_env(agent == Agent::Zed)
            .build();
        let written = followed(&place.path)?;
        let original = read_text(&written)?;
        let edited = match &place.format {
            Format::Json(table, dialect) => edit_json(
                original.as_deref(),
                &JsonPlace::builder()
                    .path(&place.path)
                    .table(table.clone())
                    .dialect(*dialect)
                    .build(),
                name,
                &entry,
                change,
                force,
            ),
            Format::Toml => edit_toml(
                original.as_deref(),
                &place.path,
                name,
                &entry,
                change,
                force,
            ),
        }?;
        let note = match (agent, scope, edited.outcome()) {
            (_, _, EditOutcome::Removed | EditOutcome::Absent) => None,
            (Agent::Claude, McpScope::Project, _) => Some(CLAUDE_PROJECT_NOTE),
            (Agent::Codex, McpScope::Project, _) => Some(CODEX_PROJECT_NOTE),
            _ => None,
        };
        Ok(PlannedEdit::builder()
            .target(target.clone())
            .path(place.path)
            .written(written)
            .outcome(edited.outcome())
            .original(original)
            .text(edited.text().clone())
            .makes_folder(place.makes_folder)
            .note(note)
            .build())
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs::{self, Permissions},
        os::unix::fs::{PermissionsExt, symlink},
        path::Path,
    };

    use serde_json::{Value, json};
    use tempfile::TempDir;

    use super::*;
    use crate::{adapter::agent_config::DEFAULT_SERVER_NAME, domain::value::ProjectRoot};

    /// Mode bits a permission check compares.
    const PERMISSION_BITS: u32 = 0o777;
    /// A private file's mode, as Claude Code keeps `.claude.json`.
    const PRIVATE_MODE: u32 = 0o600;
    /// A mode anyone may execute.
    const EXECUTABLE_MODE: u32 = 0o755;

    /// A scratch home with a project folder, and the configs under it.
    struct Home {
        /// The scratch folder.
        dir: TempDir,
    }

    impl Home {
        /// A fresh scratch home holding `project/`.
        fn new() -> Self {
            let dir = TempDir::new().unwrap();
            fs::create_dir(dir.path().join("project")).unwrap();
            Self { dir }
        }

        /// A path under the scratch home.
        fn at(&self, relative: &str) -> PathBuf {
            self.dir.path().join(relative)
        }

        /// The configs of this home, launching `/bin/muster mcp`.
        fn files(&self) -> AgentConfigFiles {
            let locations = ConfigLocations::from_env(self.dir.path(), |name| {
                (name == "PATH").then(|| self.at("bin").into_os_string())
            });
            AgentConfigFiles::new(
                Some(locations),
                ServerLaunch::builder()
                    .command(PathBuf::from("/bin/muster"))
                    .args(vec!["mcp".to_owned()])
                    .build(),
            )
        }

        /// `agent`'s config of `scope`, for the scratch project.
        fn target(&self, agent: Agent, scope: McpScope) -> ConfigTarget {
            ConfigTarget::builder()
                .agent(agent)
                .scope(scope)
                .root(Some(ProjectRoot::try_new(self.at("project")).unwrap()))
                .build()
        }

        /// Plans `change` to `agent`'s `scope` config and commits it.
        fn apply(
            &self,
            agent: Agent,
            scope: McpScope,
            change: EntryChange,
            force: bool,
        ) -> Result<PlannedEdit, AgentConfigError> {
            let edit = self
                .files()
                .plan(&self.target(agent, scope), &muster(), change, force)?;
            edit.commit()?;
            Ok(edit)
        }
    }

    /// The default server name.
    fn muster() -> McpServerName {
        McpServerName::try_new(DEFAULT_SERVER_NAME).unwrap()
    }

    /// The JSON file at `path`.
    fn json_at(path: &Path) -> Value {
        serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
    }

    /// Each agent and scope lands in the file its docs name, at the key they
    /// name, and a second add writes nothing.
    #[test]
    fn each_scope_writes_the_file_its_agent_reads() {
        let home = Home::new();
        fs::write(home.at(".claude.json"), "{\"numStartups\": 3}\n").unwrap();
        let project = home.at("project").display().to_string();
        let ours = json!({"command": "/bin/muster", "args": ["mcp"]});

        for scope in [McpScope::Local, McpScope::Project, McpScope::User] {
            let edit = home
                .apply(Agent::Claude, scope, EntryChange::Add, false)
                .unwrap();
            assert_eq!(edit.outcome(), EditOutcome::Added, "{scope}");
            let again = home
                .apply(Agent::Claude, scope, EntryChange::Add, false)
                .unwrap();
            assert_eq!(again.outcome(), EditOutcome::AlreadyAdded, "{scope}");
        }
        let claude = json_at(&home.at(".claude.json"));
        assert_eq!(claude["mcpServers"]["muster"], ours);
        assert_eq!(claude["projects"][&project]["mcpServers"]["muster"], ours);
        assert_eq!(claude["numStartups"], 3);
        assert_eq!(
            json_at(&home.at("project/.mcp.json"))["mcpServers"]["muster"],
            ours
        );

        home.apply(Agent::Codex, McpScope::User, EntryChange::Add, false)
            .unwrap();
        let codex = home
            .apply(Agent::Codex, McpScope::Project, EntryChange::Add, false)
            .unwrap();
        assert_eq!(codex.note(), Some(CODEX_PROJECT_NOTE));
        for config in [".codex/config.toml", "project/.codex/config.toml"] {
            assert!(
                fs::read_to_string(home.at(config))
                    .unwrap()
                    .contains("[mcp_servers.muster]"),
                "{config}"
            );
        }

        home.apply(Agent::Zed, McpScope::User, EntryChange::Add, false)
            .unwrap();
        assert_eq!(
            json_at(&home.at(".config/zed/settings.json"))["context_servers"]["muster"],
            json!({"command": "/bin/muster", "args": ["mcp"], "env": {}})
        );
    }

    /// Codex and Zed refuse the scopes their docs do not give them, before
    /// any file is touched.
    #[test]
    fn a_scope_the_agent_lacks_is_refused() {
        let home = Home::new();
        for (agent, scope) in [
            (Agent::Codex, McpScope::Local),
            (Agent::Zed, McpScope::Project),
            (Agent::Zed, McpScope::Local),
        ] {
            assert!(matches!(
                home.apply(agent, scope, EntryChange::Add, false),
                Err(AgentConfigError::UnsupportedScope { .. })
            ));
        }
        assert!(!home.at(".codex").exists() && !home.at(".config").exists());
        let unrooted = ConfigTarget::builder()
            .agent(Agent::Claude)
            .scope(McpScope::Project)
            .root(None)
            .build();
        assert!(matches!(
            home.files()
                .plan(&unrooted, &muster(), EntryChange::Add, false),
            Err(AgentConfigError::NoProject { .. })
        ));
    }

    /// A rewrite keeps the file's mode, and a linked dotfile stays a link
    /// with its target rewritten.
    #[test]
    fn a_rewrite_keeps_the_mode_and_the_link() {
        let home = Home::new();
        let claude = home.at(".claude.json");
        fs::write(&claude, "{}\n").unwrap();
        fs::set_permissions(&claude, Permissions::from_mode(PRIVATE_MODE)).unwrap();
        fs::create_dir_all(home.at("dotfiles/zed")).unwrap();
        fs::create_dir_all(home.at(".config/zed")).unwrap();
        fs::write(home.at("dotfiles/zed/settings.json"), "// mine\n{}\n").unwrap();
        symlink(
            home.at("dotfiles/zed/settings.json"),
            home.at(".config/zed/settings.json"),
        )
        .unwrap();

        home.apply(Agent::Claude, McpScope::User, EntryChange::Add, false)
            .unwrap();
        home.apply(Agent::Zed, McpScope::User, EntryChange::Add, false)
            .unwrap();

        let mode = fs::metadata(&claude).unwrap().permissions().mode() & PERMISSION_BITS;
        assert_eq!(mode, PRIVATE_MODE);
        assert!(
            fs::symlink_metadata(home.at(".config/zed/settings.json"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        let target = fs::read_to_string(home.at("dotfiles/zed/settings.json")).unwrap();
        assert!(
            target.starts_with("// mine\n") && target.contains("/bin/muster"),
            "{target}"
        );
    }

    /// A file changed between the plan and the write is left as the other
    /// writer left it.
    #[test]
    fn a_file_changed_after_planning_is_not_overwritten() {
        let home = Home::new();
        let claude = home.at(".claude.json");
        fs::write(&claude, "{}\n").unwrap();
        let edit = home
            .files()
            .plan(
                &home.target(Agent::Claude, McpScope::User),
                &muster(),
                EntryChange::Add,
                false,
            )
            .unwrap();
        fs::write(&claude, "{\"numStartups\": 4}\n").unwrap();

        assert!(matches!(edit.commit(), Err(AgentConfigError::Changed(_))));
        assert_eq!(
            fs::read_to_string(&claude).unwrap(),
            "{\"numStartups\": 4}\n"
        );
    }

    /// A Claude Code file with a comment is not JSON Claude Code reads, so
    /// it is refused and left byte for byte.
    #[test]
    fn an_unparseable_file_is_left_alone() {
        let home = Home::new();
        let broken = "{\"mcpServers\": {\n";
        fs::write(home.at("project/.mcp.json"), broken).unwrap();
        fs::create_dir_all(home.at(".codex")).unwrap();
        fs::write(home.at(".codex/config.toml"), "model = \n").unwrap();

        assert!(matches!(
            home.apply(Agent::Claude, McpScope::Project, EntryChange::Add, true),
            Err(AgentConfigError::Unparseable { .. })
        ));
        assert!(matches!(
            home.apply(Agent::Codex, McpScope::User, EntryChange::Add, true),
            Err(AgentConfigError::Unparseable { .. })
        ));
        assert_eq!(
            fs::read_to_string(home.at("project/.mcp.json")).unwrap(),
            broken
        );
        assert_eq!(
            fs::read_to_string(home.at(".codex/config.toml")).unwrap(),
            "model = \n"
        );
    }

    /// An agent counts as present by its config or its binary, and none is
    /// without a home folder.
    #[test]
    fn agents_are_found_by_config_or_binary() {
        let home = Home::new();
        assert!(home.files().present().is_empty());

        fs::write(home.at(".claude.json"), "{}").unwrap();
        fs::create_dir_all(home.at("bin")).unwrap();
        fs::write(home.at("bin/zeditor"), "").unwrap();
        fs::set_permissions(
            home.at("bin/zeditor"),
            Permissions::from_mode(EXECUTABLE_MODE),
        )
        .unwrap();
        assert_eq!(home.files().present(), [Agent::Claude, Agent::Zed]);

        let homeless = AgentConfigFiles::new(None, home.files().launch.clone());
        assert!(homeless.present().is_empty());
        assert!(matches!(
            homeless.plan(
                &home.target(Agent::Zed, McpScope::User),
                &muster(),
                EntryChange::Add,
                false
            ),
            Err(AgentConfigError::NoHome(Agent::Zed))
        ));
    }
}
