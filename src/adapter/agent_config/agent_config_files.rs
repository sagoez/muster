use std::path::{Path, PathBuf};

use strum::IntoEnumIterator;

use super::{
    agent::Agent,
    agent_config_error::AgentConfigError,
    claude_project_key::claude_project_key,
    config_file::{confine, is_read_only, read_text, resolved},
    config_format::ConfigFormat,
    config_locations::ConfigLocations,
    config_lock::ConfigLock,
    config_place::ConfigPlace,
    config_target::ConfigTarget,
    config_text::ConfigText,
    disabling::Disabling,
    edit_outcome::EditOutcome,
    edit_plan::EditPlan,
    edited::Edited,
    entry_change::EntryChange,
    file_edit::FileEdit,
    json_dialect::JsonDialect,
    json_document::edit_json,
    json_place::JsonPlace,
    lock_timing::LockTiming,
    mcp_registry::McpRegistry,
    mcp_scope::McpScope,
    planned_edit::PlannedEdit,
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
/// What a person should know about muster's entry turned off.
const DISABLED_NOTE: &str = "the entry is turned off; pass --force to turn it back on";

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
    /// Returns [`AgentConfigError::UnsupportedScope`] for a scope the agent
    /// lacks, [`AgentConfigError::NoProject`] for a project's config with no
    /// project, [`AgentConfigError::ProjectNotUtf8`] for a project Claude
    /// Code's config cannot name, and [`AgentConfigError::NoHome`] for a
    /// user's config with no home folder.
    fn place(&self, target: &ConfigTarget) -> Result<ConfigPlace, AgentConfigError> {
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
        let json = |keys: &[&str], dialect| {
            ConfigFormat::Json(keys.iter().map(ToString::to_string).collect(), dialect)
        };
        // Claude Code turns a server off outside its entry, in lists muster
        // leaves to Claude Code's own `/mcp`.
        let disabling = if agent == Agent::Claude {
            Disabling::Elsewhere
        } else {
            Disabling::Flag
        };
        let user_file = |path: PathBuf, format| {
            ConfigPlace::builder()
                .path(path)
                .format(format)
                .makes_folder(true)
                .locked(agent == Agent::Claude)
                .confined_to(None)
                .disabling(disabling)
                .build()
        };
        let project_file = |root: PathBuf, path: PathBuf, format| {
            ConfigPlace::builder()
                .path(path)
                .format(format)
                .makes_folder(agent == Agent::Codex)
                .locked(false)
                .confined_to(Some(root))
                .disabling(disabling)
                .build()
        };
        Ok(match (agent, scope) {
            (Agent::Claude, McpScope::Local) => {
                let home = home()?;
                let key = claude_project_key(&root()?);
                let key = key
                    .to_str()
                    .ok_or_else(|| AgentConfigError::ProjectNotUtf8(key.clone()))?;
                user_file(
                    home.claude_json().clone(),
                    json(&[CLAUDE_PROJECTS, key, CLAUDE_SERVERS], JsonDialect::Strict),
                )
            },
            (Agent::Claude, McpScope::Project) => {
                let root = root()?;
                let path = root.join(CLAUDE_PROJECT_FILE);
                project_file(root, path, json(&[CLAUDE_SERVERS], JsonDialect::Strict))
            },
            (Agent::Claude, McpScope::User) => user_file(
                home()?.claude_json().clone(),
                json(&[CLAUDE_SERVERS], JsonDialect::Strict),
            ),
            (Agent::Codex, McpScope::User) => {
                user_file(home()?.codex_home().join(CODEX_CONFIG), ConfigFormat::Toml)
            },
            (Agent::Codex, McpScope::Project) => {
                let root = root()?;
                let path = root.join(CODEX_PROJECT_DIR).join(CODEX_CONFIG);
                project_file(root, path, ConfigFormat::Toml)
            },
            (Agent::Zed, McpScope::User) => user_file(
                home()?.zed_settings().clone(),
                json(&[ZED_SERVERS], JsonDialect::WithComments),
            ),
            (Agent::Codex | Agent::Zed, _) => {
                return Err(AgentConfigError::UnsupportedScope { agent, scope });
            },
        })
    }

    /// muster's entry in `target`'s config.
    ///
    /// # Errors
    /// Returns [`AgentConfigError::CommandNotUtf8`] for a binary path a
    /// config cannot hold.
    fn entry(&self, target: &ConfigTarget) -> Result<ServerEntry, AgentConfigError> {
        self.launch
            .entry(target.scope(), target.agent() == Agent::Zed)
    }

    /// `change` worked out against `target`'s config at `place`, read from
    /// `written`, the file its path resolves to: the text as read, or none
    /// without a file, and the edit. The one read and judgement both the
    /// remove picker and a plan make.
    ///
    /// # Errors
    /// Returns the [`AgentConfigError`] of a file that is not a regular
    /// file, cannot be read or parsed, or holds an entry muster must not
    /// touch.
    fn edit_config(
        &self,
        target: &ConfigTarget,
        place: &ConfigPlace,
        written: &Path,
        name: &McpServerName,
        change: EntryChange,
        force: bool,
    ) -> Result<(Option<ConfigText>, Edited), AgentConfigError> {
        let read = read_text(written, place.path())?;
        let text = read.as_ref().map(|read| read.text().as_str());
        let entry = self.entry(target)?;
        let edited = edit_entry(place, text, name, &entry, change, force)?;
        Ok((read, edited))
    }

    /// The edit of `target`'s config at `place`, read from `written`,
    /// checked to be one muster may write.
    ///
    /// # Errors
    /// As [`Self::edit_config`], or [`AgentConfigError::ReadOnly`] for a
    /// read-only file the edit would replace.
    fn plan_one(
        &self,
        target: &ConfigTarget,
        place: &ConfigPlace,
        written: PathBuf,
        name: &McpServerName,
        change: EntryChange,
        force: bool,
    ) -> Result<PlannedEdit, AgentConfigError> {
        let (original, edited) = self.edit_config(target, place, &written, name, change, force)?;
        let outcome = edited.outcome();
        let file = match edited.text() {
            Some(_) if is_read_only(&written) => {
                return Err(AgentConfigError::ReadOnly(place.path().clone()));
            },
            Some(text) => Some(
                FileEdit::builder()
                    .path(place.path().clone())
                    .written(written)
                    .original(original)
                    .text(text.clone())
                    .makes_folder(place.makes_folder())
                    .build(),
            ),
            None => None,
        };
        let note = match (target.agent(), target.scope(), outcome) {
            (_, _, EditOutcome::Disabled) => Some(DISABLED_NOTE),
            (_, _, EditOutcome::Removed | EditOutcome::Absent) => None,
            (Agent::Claude, McpScope::Project, _) => Some(CLAUDE_PROJECT_NOTE),
            (Agent::Codex, McpScope::Project, _) => Some(CODEX_PROJECT_NOTE),
            _ => None,
        };
        Ok(PlannedEdit::builder()
            .target(target.clone())
            .path(place.path().clone())
            .outcome(outcome)
            .file(file)
            .note(note)
            .build())
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

/// The file the agent's `path` leads to once resolved, refused when it
/// leaves `confined_to`.
///
/// # Errors
/// Returns the [`AgentConfigError`] of a path that cannot be resolved or
/// leads out of its project.
fn located(path: &Path, confined_to: Option<&Path>) -> Result<PathBuf, AgentConfigError> {
    let written = resolved(path)?;
    if let Some(root) = confined_to {
        confine(&written, root, path)?;
    }
    Ok(written)
}

/// Works out `change` to the entry `name` in `text`, the config at `place`,
/// muster's own being `entry`.
///
/// # Errors
/// Returns the [`AgentConfigError`] of a file that does not parse, or holds
/// an entry muster must not touch.
fn edit_entry(
    place: &ConfigPlace,
    text: Option<&str>,
    name: &McpServerName,
    entry: &ServerEntry,
    change: EntryChange,
    force: bool,
) -> Result<Edited, AgentConfigError> {
    let path = place.path();
    match place.format() {
        ConfigFormat::Json(table, dialect) => edit_json(
            text,
            &JsonPlace::builder()
                .path(path)
                .table(table.clone())
                .dialect(*dialect)
                .build(),
            name,
            entry,
            place.disabling(),
            change,
            force,
        ),
        ConfigFormat::Toml => edit_toml(text, path, name, entry, change, force),
    }
}

impl McpRegistry for AgentConfigFiles {
    fn present(&self) -> Vec<Agent> {
        Agent::iter()
            .filter(|agent| self.is_present(*agent))
            .collect()
    }

    fn holding(&self, targets: &[ConfigTarget], name: &McpServerName) -> Vec<Agent> {
        targets
            .iter()
            .filter(|target| {
                let removal = || -> Result<Edited, AgentConfigError> {
                    let place = self.place(target)?;
                    let written = located(place.path(), place.confined_to().as_deref())?;
                    let (_, edited) = self.edit_config(
                        target,
                        &place,
                        &written,
                        name,
                        EntryChange::Remove,
                        false,
                    )?;
                    Ok(edited)
                };
                removal().is_ok_and(|edited| edited.outcome() == EditOutcome::Removed)
            })
            .map(ConfigTarget::agent)
            .collect()
    }

    fn plan(
        &self,
        targets: &[ConfigTarget],
        name: &McpServerName,
        change: EntryChange,
        force: bool,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<EditPlan, AgentConfigError> {
        let places = targets
            .iter()
            .map(|target| self.place(target))
            .collect::<Result<Vec<_>, _>>()?;
        let written = places
            .iter()
            .map(|place| located(place.path(), place.confined_to().as_deref()))
            .collect::<Result<Vec<_>, _>>()?;
        // Claude Code's lock is named for the file its path resolves to, so
        // a linked `.claude.json` is locked where Claude Code locks it.
        let mut locked: Vec<&Path> = places
            .iter()
            .zip(&written)
            .filter(|(place, _)| place.locked())
            .map(|(_, written)| written.as_path())
            .collect();
        locked.sort_unstable();
        locked.dedup();
        let timing = LockTiming::claude_code();
        let mut locks = Vec::with_capacity(locked.len());
        for path in locked {
            locks.extend(ConfigLock::take(path, &timing, cancelled)?);
        }
        let edits = targets
            .iter()
            .zip(&places)
            .zip(written)
            .map(|((target, place), written)| {
                self.plan_one(target, place, written, name, change, force)
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(EditPlan::builder().edits(edits).locks(locks).build())
    }

    fn commit(&self, plan: EditPlan, cancelled: &dyn Fn() -> bool) -> Result<(), AgentConfigError> {
        plan.commit(cancelled)
    }
}

#[cfg(test)]
mod tests {
    use std::{
        ffi::OsStr,
        fs::{self, Permissions},
        os::unix::{
            ffi::OsStrExt,
            fs::{PermissionsExt, symlink},
        },
        thread,
        time::{Duration, SystemTime},
    };

    use serde_json::json;

    use super::*;
    use crate::{
        adapter::{
            agent_config::{
                CLAUDE_CONFIG_DIR_ENV,
                testing::{Home, MUSTER_BINARY, json_at, muster, uncancelled},
            },
            test_fifo::make_fifo,
        },
        domain::value::ProjectRoot,
    };

    /// Mode bits a permission check compares.
    const PERMISSION_BITS: u32 = 0o777;
    /// A private file's mode, as Claude Code keeps `.claude.json`.
    const PRIVATE_MODE: u32 = 0o600;
    /// A mode anyone may read.
    const SHARED_MODE: u32 = 0o644;
    /// A mode nobody may write.
    const READ_ONLY_MODE: u32 = 0o444;
    /// A folder's mode nobody may write into.
    const SEALED_FOLDER_MODE: u32 = 0o555;
    /// A folder's usual mode.
    const OPEN_FOLDER_MODE: u32 = 0o755;
    /// A mode anyone may execute.
    const EXECUTABLE_MODE: u32 = 0o755;
    /// How long a test holds Claude Code's lock before letting it go.
    const LOCK_HELD_FOR: Duration = Duration::from_millis(300);
    /// How far ahead another writer that took the lock over stamps it.
    const LOCK_TAKEN_AHEAD: Duration = Duration::from_secs(60);

    /// The permission bits of the file at `path`.
    fn mode(path: &Path) -> u32 {
        fs::metadata(path).unwrap().permissions().mode() & PERMISSION_BITS
    }

    /// Plans `change` without force for each of `targets` in `home`.
    fn planned(
        home: &Home,
        targets: &[ConfigTarget],
        change: EntryChange,
    ) -> Result<EditPlan, AgentConfigError> {
        home.files()
            .plan(targets, &muster(), change, false, &uncancelled)
    }

    /// The names in `folder` besides `kept`.
    fn strays(folder: &Path, kept: &[&str]) -> Vec<String> {
        fs::read_dir(folder)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| !kept.contains(&name.as_str()))
            .collect()
    }

    /// Each agent and scope lands in the file its docs name, at the key they
    /// name, and a second add writes nothing.
    #[test]
    fn each_scope_writes_the_file_its_agent_reads() {
        let home = Home::new();
        fs::write(home.at(".claude.json"), "{\"numStartups\": 3}\n").unwrap();
        let project = home.at("project").display().to_string();
        let ours = json!({"command": MUSTER_BINARY, "args": ["mcp"]});

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
            json!({"command": "muster", "args": ["mcp"]}),
            "a file shared with the project names muster for each person's PATH"
        );

        home.apply(Agent::Codex, McpScope::User, EntryChange::Add, false)
            .unwrap();
        let codex = home
            .apply(Agent::Codex, McpScope::Project, EntryChange::Add, false)
            .unwrap();
        assert_eq!(codex.note(), Some(CODEX_PROJECT_NOTE));
        assert!(
            fs::read_to_string(home.at(".codex/config.toml"))
                .unwrap()
                .contains(&format!("command = \"{MUSTER_BINARY}\""))
        );
        assert!(
            fs::read_to_string(home.at("project/.codex/config.toml"))
                .unwrap()
                .contains("[mcp_servers.muster]\ncommand = \"muster\"\n")
        );

        home.apply(Agent::Zed, McpScope::User, EntryChange::Add, false)
            .unwrap();
        assert_eq!(
            json_at(home.locations().zed_settings())["context_servers"]["muster"],
            json!({"command": MUSTER_BINARY, "args": ["mcp"], "env": {}})
        );
    }

    /// A project in a git repository is keyed in `.claude.json` by the
    /// repository's root, as Claude Code reads it from anywhere inside.
    #[test]
    fn a_local_entry_is_keyed_by_the_repository_root() {
        let home = Home::new();
        fs::create_dir_all(home.at(".git")).unwrap();
        home.apply(Agent::Claude, McpScope::Local, EntryChange::Add, false)
            .unwrap();
        let claude = json_at(&home.at(".claude.json"));
        let repository = home.root().to_str().unwrap();
        assert!(
            claude["projects"][repository]["mcpServers"]["muster"].is_object(),
            "{claude}"
        );
    }

    /// A project folder that is not UTF-8 is refused as the key of a local
    /// entry rather than written lossily, and nothing is written.
    #[test]
    fn a_project_folder_that_is_not_utf8_is_refused() {
        let home = Home::new();
        fs::write(home.at(".claude.json"), "{}\n").unwrap();
        let folder = home.root().join(OsStr::from_bytes(b"shop-\xff"));
        let target = ConfigTarget::builder()
            .agent(Agent::Claude)
            .scope(McpScope::Local)
            .root(Some(ProjectRoot::try_new(folder).unwrap()))
            .build();
        assert!(matches!(
            planned(&home, &[target], EntryChange::Add),
            Err(AgentConfigError::ProjectNotUtf8(_))
        ));
        assert_eq!(fs::read_to_string(home.at(".claude.json")).unwrap(), "{}\n");
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
            planned(&home, &[unrooted], EntryChange::Add),
            Err(AgentConfigError::NoProject { .. })
        ));
    }

    /// A rewrite keeps the file's mode, a linked dotfile stays a link with
    /// its target rewritten, and a link that leads nowhere yet is written at
    /// its target.
    #[test]
    fn a_rewrite_keeps_the_mode_and_the_link() {
        let home = Home::new();
        let claude = home.at(".claude.json");
        fs::write(&claude, "{}\n").unwrap();
        fs::set_permissions(&claude, Permissions::from_mode(SHARED_MODE)).unwrap();
        let zed = home.locations().zed_settings().clone();
        fs::create_dir_all(home.at("dotfiles/zed")).unwrap();
        fs::create_dir_all(zed.parent().unwrap()).unwrap();
        fs::write(home.at("dotfiles/zed/settings.json"), "// mine\n{}\n").unwrap();
        symlink(home.at("dotfiles/zed/settings.json"), &zed).unwrap();
        fs::create_dir_all(home.at("dotfiles/codex")).unwrap();
        fs::create_dir_all(home.at(".codex")).unwrap();
        symlink(
            home.at("dotfiles/codex/config.toml"),
            home.at(".codex/config.toml"),
        )
        .unwrap();

        for agent in [Agent::Claude, Agent::Zed, Agent::Codex] {
            home.apply(agent, McpScope::User, EntryChange::Add, false)
                .unwrap();
        }

        assert_eq!(
            mode(&claude),
            SHARED_MODE,
            "an existing file keeps its mode"
        );
        assert!(fs::symlink_metadata(&zed).unwrap().file_type().is_symlink());
        let target = fs::read_to_string(home.at("dotfiles/zed/settings.json")).unwrap();
        assert!(
            target.starts_with("// mine\n") && target.contains(MUSTER_BINARY),
            "{target}"
        );
        assert!(
            fs::symlink_metadata(home.at(".codex/config.toml"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert!(
            fs::read_to_string(home.at("dotfiles/codex/config.toml"))
                .unwrap()
                .contains("[mcp_servers.muster]")
        );
    }

    /// A config muster creates is readable by its owner alone, for Claude
    /// Code's account details and every other agent's file alike.
    #[test]
    fn a_created_config_is_private() {
        let home = Home::new();
        for agent in [Agent::Claude, Agent::Codex, Agent::Zed] {
            home.apply(agent, McpScope::User, EntryChange::Add, false)
                .unwrap();
        }
        home.apply(Agent::Claude, McpScope::Project, EntryChange::Add, false)
            .unwrap();
        for created in [
            home.at(".claude.json"),
            home.at(".codex/config.toml"),
            home.locations().zed_settings().clone(),
            home.at("project/.mcp.json"),
        ] {
            assert_eq!(mode(&created), PRIVATE_MODE, "{}", created.display());
        }
    }

    /// A read-only config is refused, not replaced, unless the edit would
    /// write nothing.
    #[test]
    fn a_read_only_config_is_refused() {
        let home = Home::new();
        let claude = home.at(".claude.json");
        fs::write(&claude, "{}\n").unwrap();
        fs::set_permissions(&claude, Permissions::from_mode(READ_ONLY_MODE)).unwrap();
        assert!(matches!(
            home.apply(Agent::Claude, McpScope::User, EntryChange::Add, false),
            Err(AgentConfigError::ReadOnly(_))
        ));
        assert_eq!(fs::read_to_string(&claude).unwrap(), "{}\n");
        assert_eq!(
            home.apply(Agent::Claude, McpScope::User, EntryChange::Remove, false)
                .unwrap()
                .outcome(),
            EditOutcome::Absent
        );
    }

    /// A file changed between the plan and the write is left as the other
    /// writer left it, and so is every other config of the run.
    #[test]
    fn a_file_changed_after_planning_stops_every_write() {
        let home = Home::new();
        let claude = home.at(".claude.json");
        fs::write(&claude, "{}\n").unwrap();
        let plan = planned(
            &home,
            &[
                home.target(Agent::Codex, McpScope::User),
                home.target(Agent::Claude, McpScope::User),
            ],
            EntryChange::Add,
        )
        .unwrap();
        fs::write(&claude, "{\"numStartups\": 4}\n").unwrap();

        assert!(matches!(
            home.files().commit(plan, &uncancelled),
            Err(AgentConfigError::Changed(_))
        ));
        assert_eq!(
            fs::read_to_string(&claude).unwrap(),
            "{\"numStartups\": 4}\n"
        );
        assert!(!home.at(".codex").exists(), "no folder muster made is left");
    }

    /// A config that cannot be written stops the run before any other is
    /// written, and leaves no temporary file behind.
    #[test]
    fn a_write_that_fails_leaves_every_config_as_it_was() {
        let home = Home::new();
        let claude = home.at(".claude.json");
        fs::write(&claude, "{}\n").unwrap();
        let zed = home.locations().zed_settings().clone();
        let folder = zed.parent().unwrap().to_path_buf();
        fs::create_dir_all(&folder).unwrap();
        fs::write(&zed, "{}\n").unwrap();
        fs::set_permissions(&folder, Permissions::from_mode(SEALED_FOLDER_MODE)).unwrap();

        let plan = planned(
            &home,
            &[
                home.target(Agent::Claude, McpScope::User),
                home.target(Agent::Zed, McpScope::User),
            ],
            EntryChange::Add,
        )
        .unwrap();
        let failed = home.files().commit(plan, &uncancelled);
        fs::set_permissions(&folder, Permissions::from_mode(OPEN_FOLDER_MODE)).unwrap();

        assert!(
            matches!(failed, Err(AgentConfigError::Io { .. })),
            "{failed:?}"
        );
        assert_eq!(fs::read_to_string(&claude).unwrap(), "{}\n");
        assert!(
            strays(home.root(), &[".claude.json", "project", ".config"]).is_empty(),
            "{:?}",
            strays(home.root(), &[])
        );
    }

    /// A config that cannot be written takes out the folders muster made
    /// for another config of the run, so a refused run leaves no trace.
    #[test]
    fn a_failed_write_leaves_no_folder_it_made() {
        let home = Home::new();
        let codex = home.at(".codex");
        fs::create_dir_all(&codex).unwrap();
        fs::set_permissions(&codex, Permissions::from_mode(SEALED_FOLDER_MODE)).unwrap();

        let plan = planned(
            &home,
            &[
                home.target(Agent::Zed, McpScope::User),
                home.target(Agent::Codex, McpScope::User),
            ],
            EntryChange::Add,
        )
        .unwrap();
        let failed = home.files().commit(plan, &uncancelled);
        fs::set_permissions(&codex, Permissions::from_mode(OPEN_FOLDER_MODE)).unwrap();

        assert!(
            matches!(failed, Err(AgentConfigError::Io { .. })),
            "{failed:?}"
        );
        assert!(!home.at(".config").exists(), "Zed's folders are taken out");
        assert!(codex.is_dir(), "a folder muster did not make stays");
    }

    /// A rename that fails after another config was put in place names the
    /// config already written.
    #[test]
    fn a_partial_write_names_what_was_written() {
        let home = Home::new();
        let claude = home.at(".claude.json");
        let codex = home.at(".codex/config.toml");
        fs::write(&claude, "{}\n").unwrap();
        fs::create_dir_all(home.at(".codex")).unwrap();
        fs::write(&codex, "").unwrap();
        let plan = planned(
            &home,
            &[
                home.target(Agent::Claude, McpScope::User),
                home.target(Agent::Codex, McpScope::User),
            ],
            EntryChange::Add,
        )
        .unwrap();
        // Codex's file turns into a folder once both are checked, so its
        // rename fails after Claude Code's.
        let blocking = || {
            fs::remove_file(&codex).unwrap();
            fs::create_dir(&codex).unwrap();
            fs::write(codex.join("keep"), "").unwrap();
            false
        };

        let failed = home.files().commit(plan, &blocking).unwrap_err();
        let AgentConfigError::PartlyWritten { written, source } = &failed else {
            panic!("{failed:?}");
        };
        assert_eq!(written.as_slice(), [claude.as_path()]);
        assert!(source.is_io(), "{source}");
        assert!(failed.is_io());
        assert!(
            failed.to_string().contains(&claude.display().to_string()),
            "{failed}"
        );
        assert!(json_at(&claude)["mcpServers"]["muster"].is_object());
    }

    /// An interrupt before the first write leaves every config as it was
    /// and no temporary file behind.
    #[test]
    fn an_interrupt_before_writing_changes_nothing() {
        let home = Home::new();
        let claude = home.at(".claude.json");
        fs::write(&claude, "{}\n").unwrap();
        let plan = planned(
            &home,
            &[home.target(Agent::Claude, McpScope::User)],
            EntryChange::Add,
        )
        .unwrap();

        assert!(matches!(
            home.files().commit(plan, &|| true),
            Err(AgentConfigError::Interrupted)
        ));
        assert_eq!(fs::read_to_string(&claude).unwrap(), "{}\n");
        assert!(strays(home.root(), &[".claude.json", "project"]).is_empty());
    }

    /// Claude Code's lock is held from the plan's read to the write, a
    /// held one is waited for, and a stale one is taken over.
    #[test]
    fn claude_codes_lock_guards_the_edit() {
        let home = Home::new();
        let claude = home.at(".claude.json");
        let lock = home.at(".claude.json.lock");
        fs::write(&claude, "{}\n").unwrap();

        let plan = planned(
            &home,
            &[home.target(Agent::Claude, McpScope::User)],
            EntryChange::Add,
        )
        .unwrap();
        assert!(lock.is_dir(), "held until the write");
        home.files().commit(plan, &uncancelled).unwrap();
        assert!(!lock.exists(), "let go after the write");

        fs::create_dir(&lock).unwrap();
        let held = lock.clone();
        let release = thread::spawn(move || {
            thread::sleep(LOCK_HELD_FOR);
            fs::remove_dir(held).unwrap();
        });
        let removed = home
            .apply(Agent::Claude, McpScope::User, EntryChange::Remove, false)
            .unwrap();
        release.join().unwrap();
        assert_eq!(removed.outcome(), EditOutcome::Removed);
        assert!(!lock.exists());
    }

    /// `.claude.json` is read only once its lock is taken, so a write
    /// Claude Code makes while muster waits for the lock is kept, not lost
    /// or refused.
    #[test]
    fn claude_json_is_read_under_its_lock() {
        let home = Home::new();
        let claude = home.at(".claude.json");
        let lock = home.at(".claude.json.lock");
        fs::write(&claude, "{}\n").unwrap();
        fs::create_dir(&lock).unwrap();
        let (written, held) = (claude.clone(), lock.clone());
        let writer = thread::spawn(move || {
            thread::sleep(LOCK_HELD_FOR);
            fs::write(written, "{\"numStartups\": 5}\n").unwrap();
            fs::remove_dir(held).unwrap();
        });

        let added = home.apply(Agent::Claude, McpScope::User, EntryChange::Add, false);
        writer.join().unwrap();
        assert_eq!(added.unwrap().outcome(), EditOutcome::Added);
        let after = json_at(&claude);
        assert_eq!(after["numStartups"], 5);
        assert!(after["mcpServers"]["muster"].is_object(), "{after}");
        assert!(!lock.exists(), "let go after the write");
    }

    /// A rewrite between the plan and the write is caught by the file's
    /// text even when it keeps the size and the modified time, as a coarse
    /// clock leaves them, and nothing is written.
    #[test]
    fn a_same_size_rewrite_is_caught_by_its_text() {
        let home = Home::new();
        let codex = home.at(".codex/config.toml");
        fs::create_dir_all(codex.parent().unwrap()).unwrap();
        fs::write(&codex, "model = \"o3\"\n").unwrap();
        let before = fs::metadata(&codex).unwrap().modified().unwrap();
        let plan = planned(
            &home,
            &[home.target(Agent::Codex, McpScope::User)],
            EntryChange::Add,
        )
        .unwrap();
        fs::write(&codex, "model = \"o4\"\n").unwrap();
        fs::File::options()
            .write(true)
            .open(&codex)
            .unwrap()
            .set_modified(before)
            .unwrap();

        assert!(matches!(
            home.files().commit(plan, &uncancelled),
            Err(AgentConfigError::Changed(_))
        ));
        assert_eq!(fs::read_to_string(&codex).unwrap(), "model = \"o4\"\n");
    }

    /// A linked `.claude.json` is locked where its link leads, as Claude
    /// Code's lock resolves it, and the lock is let go after the write.
    #[test]
    fn a_linked_claude_json_is_locked_at_its_target() {
        let home = Home::new();
        let target = home.at("dotfiles/claude.json");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(&target, "{}\n").unwrap();
        symlink(&target, home.at(".claude.json")).unwrap();

        let plan = planned(
            &home,
            &[home.target(Agent::Claude, McpScope::User)],
            EntryChange::Add,
        )
        .unwrap();
        assert!(home.at("dotfiles/claude.json.lock").is_dir());
        assert!(!home.at(".claude.json.lock").exists());
        home.files().commit(plan, &uncancelled).unwrap();
        assert!(!home.at("dotfiles/claude.json.lock").exists());
        assert!(
            fs::symlink_metadata(home.at(".claude.json"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert!(json_at(&target)["mcpServers"]["muster"].is_object());
    }

    /// A lock another writer took over before the write stops it, and
    /// every config of the run is left as it was.
    #[test]
    fn a_lock_lost_before_the_write_writes_nothing() {
        let home = Home::new();
        let claude = home.at(".claude.json");
        let lock = home.at(".claude.json.lock");
        fs::write(&claude, "{}\n").unwrap();
        let plan = planned(
            &home,
            &[
                home.target(Agent::Codex, McpScope::User),
                home.target(Agent::Claude, McpScope::User),
            ],
            EntryChange::Add,
        )
        .unwrap();
        fs::File::open(&lock)
            .unwrap()
            .set_modified(SystemTime::now() + LOCK_TAKEN_AHEAD)
            .unwrap();

        assert!(matches!(
            home.files().commit(plan, &uncancelled),
            Err(AgentConfigError::LockLost(_))
        ));
        assert_eq!(fs::read_to_string(&claude).unwrap(), "{}\n");
        assert!(!home.at(".codex").exists());
        assert!(lock.is_dir(), "the new holder's lock is kept");
    }

    /// A project's file that is a pipe, a device or a link out of the
    /// project is refused without being read or written.
    #[test]
    fn a_project_file_must_be_a_regular_file_inside_it() {
        let home = Home::new();
        let mcp_json = home.at("project/.mcp.json");
        make_fifo(&mcp_json);
        assert!(matches!(
            home.apply(Agent::Claude, McpScope::Project, EntryChange::Add, false),
            Err(AgentConfigError::NotAFile(_))
        ));
        fs::remove_file(&mcp_json).unwrap();
        symlink("/dev/zero", &mcp_json).unwrap();
        assert!(matches!(
            home.apply(Agent::Claude, McpScope::Project, EntryChange::Add, false),
            Err(AgentConfigError::OutsideProject { .. })
        ));
        fs::remove_file(&mcp_json).unwrap();

        let outside = home.at("outside.json");
        fs::write(&outside, "{}\n").unwrap();
        symlink("../outside.json", &mcp_json).unwrap();
        assert!(matches!(
            home.apply(Agent::Claude, McpScope::Project, EntryChange::Add, false),
            Err(AgentConfigError::OutsideProject { .. })
        ));
        assert_eq!(fs::read_to_string(&outside).unwrap(), "{}\n");

        fs::create_dir_all(home.at("elsewhere")).unwrap();
        symlink("../elsewhere", home.at("project/.codex")).unwrap();
        assert!(matches!(
            home.apply(Agent::Codex, McpScope::Project, EntryChange::Add, false),
            Err(AgentConfigError::OutsideProject { .. })
        ));
        assert!(!home.at("elsewhere/config.toml").exists());
    }

    /// A project's file linked through a folder not made yet and back out
    /// of the project is refused, and no folder is made inside or outside.
    #[test]
    fn a_link_out_past_a_missing_folder_is_refused() {
        let home = Home::new();
        fs::create_dir_all(home.at("project/.codex")).unwrap();
        symlink(
            "missing/../../../victim/config.toml",
            home.at("project/.codex/config.toml"),
        )
        .unwrap();
        for force in [false, true] {
            assert!(matches!(
                home.apply(Agent::Codex, McpScope::Project, EntryChange::Add, force),
                Err(AgentConfigError::OutsideProject { .. })
            ));
        }
        assert!(!home.at("victim").exists());
        assert!(!home.at("project/.codex/missing").exists());
    }

    /// The remove picker ticks only the agents whose config holds muster's
    /// own entry under the name, so its default choice removes them all
    /// without --force; another server's entry is left unticked.
    #[test]
    fn only_configs_with_musters_entry_are_holding() {
        let home = Home::new();
        home.apply(Agent::Codex, McpScope::User, EntryChange::Add, false)
            .unwrap();
        fs::write(
            home.at(".claude.json"),
            "{\"mcpServers\": {\"muster\": {\"command\": \"other\"}}}",
        )
        .unwrap();
        let targets: Vec<ConfigTarget> = Agent::iter()
            .map(|agent| home.target(agent, McpScope::User))
            .collect();
        let holding = home.files().holding(&targets, &muster());
        assert_eq!(holding, [Agent::Codex]);

        let ticked: Vec<ConfigTarget> = holding
            .iter()
            .map(|agent| home.target(*agent, McpScope::User))
            .collect();
        let plan = planned(&home, &ticked, EntryChange::Remove).unwrap();
        assert_eq!(plan.edits()[0].outcome(), EditOutcome::Removed);
    }

    /// muster leaves Claude Code's lists of turned-off servers to Claude
    /// Code: an entry a project's list names reads as added, with or
    /// without `--force`, and `.claude.json` is left as it was.
    #[test]
    fn claude_codes_turned_off_lists_are_left_alone() {
        let home = Home::new();
        let claude = home.at(".claude.json");
        let project = home.at("project").display().to_string();
        let ours = json!({"command": MUSTER_BINARY, "args": ["mcp"]});
        let off = json!({"numStartups": 3, "projects": {&project: {
            "mcpServers": {"muster": ours},
            "disabledMcpServers": ["other", "muster"],
        }}});
        fs::write(&claude, off.to_string()).unwrap();

        for force in [false, true] {
            let kept = home
                .apply(Agent::Claude, McpScope::Local, EntryChange::Add, force)
                .unwrap();
            assert_eq!(kept.outcome(), EditOutcome::AlreadyAdded);
            assert_eq!(kept.note(), None);
        }
        assert_eq!(json_at(&claude), off);
    }

    /// A project's `.mcp.json` is planned without a home folder, and no
    /// user file is read: a broken `.claude.json` or settings file of the
    /// user does not stand in the way.
    #[test]
    fn a_project_scope_reads_no_user_file() {
        let home = Home::new();
        fs::write(home.at(".claude.json"), "{ broken").unwrap();
        fs::create_dir_all(home.at(".claude")).unwrap();
        fs::write(home.at(".claude/settings.json"), "{ broken").unwrap();
        let added = home
            .apply(Agent::Claude, McpScope::Project, EntryChange::Add, false)
            .unwrap();
        assert_eq!(added.outcome(), EditOutcome::Added);
        assert!(!home.at(".claude.json.lock").exists());

        let homeless = AgentConfigFiles::new(None, home.files().launch.clone());
        let plan = homeless
            .plan(
                &[home.target(Agent::Claude, McpScope::Project)],
                &muster(),
                EntryChange::Remove,
                false,
                &uncancelled,
            )
            .unwrap();
        assert_eq!(plan.edits()[0].outcome(), EditOutcome::Removed);
    }

    /// Codex's and Zed's entries are off by their own `enabled = false`, and
    /// `--force` clears it in place, keeping every other key.
    #[test]
    fn codex_and_zed_turn_an_entry_off_by_its_flag() {
        let home = Home::new();
        let codex = home.at(".codex/config.toml");
        fs::create_dir_all(codex.parent().unwrap()).unwrap();
        fs::write(
            &codex,
            format!(
                "[mcp_servers.muster]\ncommand = \"{MUSTER_BINARY}\"\nargs = [\"mcp\"]\nenabled = false\nstartup_timeout_sec = 20\n"
            ),
        )
        .unwrap();
        let zed = home.locations().zed_settings().clone();
        fs::create_dir_all(zed.parent().unwrap()).unwrap();
        fs::write(
            &zed,
            format!(
                "// mine\n{{\"context_servers\": {{\"muster\": {{\"command\": \"{MUSTER_BINARY}\", \"args\": [\"mcp\"], \"env\": {{\"A\": \"1\"}}, \"enabled\": false}}}}}}\n"
            ),
        )
        .unwrap();

        for agent in [Agent::Codex, Agent::Zed] {
            let kept = home
                .apply(agent, McpScope::User, EntryChange::Add, false)
                .unwrap();
            assert_eq!(kept.outcome(), EditOutcome::Disabled, "{agent}");
            let on = home
                .apply(agent, McpScope::User, EntryChange::Add, true)
                .unwrap();
            assert_eq!(on.outcome(), EditOutcome::Enabled, "{agent}");
        }
        assert_eq!(
            fs::read_to_string(&codex).unwrap(),
            format!(
                "[mcp_servers.muster]\ncommand = \"{MUSTER_BINARY}\"\nargs = [\"mcp\"]\nstartup_timeout_sec = 20\n"
            )
        );
        let zed_text = fs::read_to_string(&zed).unwrap();
        assert!(zed_text.starts_with("// mine\n"), "{zed_text}");
        assert!(!zed_text.contains("enabled"), "{zed_text}");
        assert!(zed_text.contains("\"A\": \"1\""), "{zed_text}");
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
                &[home.target(Agent::Zed, McpScope::User)],
                &muster(),
                EntryChange::Add,
                false,
                &uncancelled
            ),
            Err(AgentConfigError::NoHome(Agent::Zed))
        ));
    }

    /// Claude Code's config folder moved by `CLAUDE_CONFIG_DIR` counts it
    /// as present, and is made when missing, as Codex's and Zed's are.
    #[test]
    fn a_moved_claude_config_folder_is_found_and_made() {
        let home = Home::new();
        let moved = home.at("claude-config");
        let locations = ConfigLocations::from_env(home.root(), |name| {
            (name == CLAUDE_CONFIG_DIR_ENV).then(|| moved.clone().into_os_string())
        });
        let files = AgentConfigFiles::new(Some(locations), home.files().launch.clone());
        assert!(files.present().is_empty());
        let plan = files
            .plan(
                &[home.target(Agent::Claude, McpScope::User)],
                &muster(),
                EntryChange::Add,
                false,
                &uncancelled,
            )
            .unwrap();
        files.commit(plan, &uncancelled).unwrap();
        assert!(json_at(&moved.join(".claude.json"))["mcpServers"]["muster"].is_object());
        assert_eq!(files.present(), [Agent::Claude]);
    }
}
