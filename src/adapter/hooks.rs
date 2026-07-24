use std::{
    env, fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

use atomic_write_file::AtomicWriteFile;
use directories::BaseDirs;
use serde_json::{Map, Value, json};
use thiserror::Error;
use toml_edit::{ArrayOfTables, DocumentMut, Item, Table};

use crate::{
    constants::MUSTER_AGENT_SESSION_ENV,
    domain::{
        agent_session::{AgentProcessId, AgentSessionId, NativeSessionId},
        port::AgentSessionStore,
        process::{AGENT_PROTOCOL_VERSION, AgentTool},
    },
};

/// Claude Code user settings relative to the user's home directory.
const CLAUDE_SETTINGS: &str = ".claude/settings.json";
/// Default Codex configuration directory relative to the user's home directory.
const CODEX_CONFIG_DIR: &str = ".codex";
/// Environment variable overriding Codex's configuration directory.
const CODEX_HOME_ENV: &str = "CODEX_HOME";
/// Gemini CLI user settings relative to the user's home directory.
const GEMINI_SETTINGS: &str = ".gemini/settings.json";
/// Copilot CLI's dedicated Muster hook relative to the user's home directory.
const COPILOT_HOOK: &str = ".copilot/hooks/muster.json";
/// Kimi Code configuration relative to the user's home directory.
const KIMI_CONFIG: &str = ".kimi-code/config.toml";
/// Amp's dedicated Muster plugin relative to the platform config directory.
const AMP_PLUGIN: &str = "amp/plugins/muster.ts";
/// OpenCode's dedicated Muster plugin relative to its XDG config directory.
const OPENCODE_PLUGIN: &str = "opencode/plugins/muster.js";
/// Environment variable controlling the XDG configuration root.
const XDG_CONFIG_HOME_ENV: &str = "XDG_CONFIG_HOME";
/// XDG configuration root used when the environment does not override it.
const XDG_CONFIG_HOME_DEFAULT: &str = ".config";
/// Lifecycle event used by JSON-configured providers.
const SESSION_START_EVENT: &str = "SessionStart";
/// Lifecycle event used by Copilot's camel-case hook format.
const COPILOT_SESSION_START_EVENT: &str = "sessionStart";
/// Kimi lifecycle matcher covering new, resumed, and reset sessions.
const KIMI_SESSION_MATCHER: &str = "startup|resume|clear";
/// Canonical lifecycle event accepted by the versioned wire protocol.
const PROTOCOL_SESSION_STARTED: &str = "session_started";
/// Hidden CLI subcommand containing provider lifecycle commands.
const HOOK_SUBCOMMAND: &str = "hook";
/// Hidden CLI action that records a provider lifecycle event.
const CAPTURE_SUBCOMMAND: &str = "capture";
/// CLI argument identifying the provider that emitted a lifecycle event.
const CAPTURE_PROVIDER_ARGUMENT: &str = "--provider";
/// CLI argument identifying the provider process that invoked a capture hook.
const CAPTURE_PROCESS_ID_ARGUMENT: &str = "--process-id";
/// CLI argument identifying the provider process's immediate parent.
const CAPTURE_PARENT_PROCESS_ID_ARGUMENT: &str = "--parent-process-id";
/// Maximum provider-config symlinks followed before treating the path as a
/// cycle or an unreasonable chain.
const MAX_PROVIDER_CONFIG_SYMLINKS: usize = 40;

/// Failures while installing or receiving provider lifecycle integrations.
#[derive(Debug, Error)]
pub enum HookError {
    /// No user directories could be resolved on this platform.
    #[error("no user configuration directory is available")]
    NoUserDirs,
    /// The running executable path cannot be represented in provider config.
    #[error("the muster executable path is not valid UTF-8")]
    InvalidExecutable,
    /// A hook payload did not contain a usable provider session identity.
    #[error("the hook payload does not contain a session ID")]
    MissingSessionId,
    /// A hook did not report a valid parent provider process ID.
    #[error("the hook did not report a valid provider process ID")]
    MissingProviderProcessId,
    /// A canonical event uses a protocol version this binary does not support.
    #[error("unsupported agent protocol version {0}")]
    UnsupportedProtocolVersion(u64),
    /// A canonical event name is not part of the supported protocol version.
    #[error("unsupported agent protocol event {0}")]
    UnsupportedProtocolEvent(String),
    /// A versioned event omitted its required event name.
    #[error("the agent protocol event name is missing")]
    MissingProtocolEvent,
    /// A file could not be read.
    #[error("could not read hook config {path}: {source}")]
    Read {
        /// File that failed to load.
        path: PathBuf,
        /// Underlying I/O error.
        source: std::io::Error,
    },
    /// A file could not be written.
    #[error("could not write hook config {path}: {source}")]
    Write {
        /// File that failed to update.
        path: PathBuf,
        /// Underlying I/O error.
        source: std::io::Error,
    },
    /// Existing JSON configuration is invalid.
    #[error("could not parse hook config {path}: {source}")]
    Json {
        /// Invalid configuration file.
        path: PathBuf,
        /// JSON parse failure.
        source: serde_json::Error,
    },
    /// Existing TOML configuration is invalid.
    #[error("could not parse hook config {path}: {source}")]
    Toml {
        /// Invalid configuration file.
        path: PathBuf,
        /// TOML parse failure.
        source: toml_edit::TomlError,
    },
    /// Existing configuration has an incompatible field shape.
    #[error("hook config {0} has an incompatible schema")]
    Schema(PathBuf),
    /// Reading hook JSON from stdin failed.
    #[error("could not read the provider hook payload: {0}")]
    PayloadRead(#[from] std::io::Error),
    /// The provider hook payload is invalid JSON.
    #[error("could not parse the provider hook payload: {0}")]
    PayloadJson(#[from] serde_json::Error),
    /// Encoding an owned provider plugin failed.
    #[error("could not encode a provider integration: {0}")]
    PluginEncoding(serde_json::Error),
    /// A provider configuration path contains a symlink cycle or excessive
    /// indirection.
    #[error("provider hook config symlink chain is too deep at {0}")]
    SymlinkDepth(PathBuf),
}

/// Installs opt-in provider integrations and receives their session identities.
pub struct ProviderHooks;

impl ProviderHooks {
    /// Installs idempotent user-level hooks/plugins for every supported provider.
    /// Returns the paths checked or updated.
    ///
    /// # Errors
    /// Returns a [`HookError`] without overwriting malformed existing configs.
    pub fn setup(executable: &Path) -> Result<Vec<PathBuf>, HookError> {
        let dirs = BaseDirs::new().ok_or(HookError::NoUserDirs)?;
        let xdg_config = Self::xdg_config_dir(dirs.home_dir());
        let codex_home = env::var_os(CODEX_HOME_ENV)
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .unwrap_or_else(|| dirs.home_dir().join(CODEX_CONFIG_DIR));
        Self::setup_in_with_codex(
            executable,
            dirs.home_dir(),
            dirs.config_dir(),
            &xdg_config,
            &codex_home,
        )
    }

    /// Captures a provider session ID from hook JSON. Hooks outside a
    /// Muster-owned agent process are deliberately ignored.
    ///
    /// # Errors
    /// Returns a [`HookError`] for malformed payloads and a config error when the
    /// session store cannot record the identity.
    pub fn capture(
        store: &dyn AgentSessionStore,
        provider: AgentTool,
        process_id: u32,
        parent_process_id: Option<u32>,
        mut input: impl Read,
    ) -> Result<bool, crate::error::MusterError> {
        let Some(internal) = std::env::var_os(MUSTER_AGENT_SESSION_ENV) else {
            return Ok(false);
        };
        let Some(internal) = internal.to_str() else {
            return Err(HookError::MissingSessionId.into());
        };
        let internal =
            AgentSessionId::try_new(internal).map_err(|_| HookError::MissingSessionId)?;
        let process_id =
            AgentProcessId::try_new(process_id).map_err(|_| HookError::MissingProviderProcessId)?;
        let parent_process_id = parent_process_id
            .map(AgentProcessId::try_new)
            .transpose()
            .map_err(|_| HookError::MissingProviderProcessId)?;
        let mut raw = String::new();
        input.read_to_string(&mut raw).map_err(HookError::from)?;
        let payload: Value = serde_json::from_str(&raw).map_err(HookError::from)?;
        let native = Self::native_id(&payload)?;
        store.capture_native_id(&internal, provider, process_id, parent_process_id, native)?;
        Ok(true)
    }

    /// Resolves OpenCode's XDG configuration root independently of the
    /// platform-native configuration directory.
    fn xdg_config_dir(home: &Path) -> PathBuf {
        env::var_os(XDG_CONFIG_HOME_ENV)
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .unwrap_or_else(|| home.join(XDG_CONFIG_HOME_DEFAULT))
    }

    /// Installs every provider integration under explicit testable roots.
    ///
    /// # Errors
    /// Returns a [`HookError`] if any existing config is malformed or a write
    /// cannot complete.
    #[cfg(test)]
    fn setup_in(
        executable: &Path,
        home: &Path,
        config: &Path,
        xdg_config: &Path,
    ) -> Result<Vec<PathBuf>, HookError> {
        Self::setup_in_with_codex(
            executable,
            home,
            config,
            xdg_config,
            &home.join(CODEX_CONFIG_DIR),
        )
    }

    /// Installs integrations with an explicit Codex configuration directory.
    ///
    /// # Errors
    /// Returns a [`HookError`] if any existing config is malformed or a write
    /// cannot complete.
    fn setup_in_with_codex(
        executable: &Path,
        home: &Path,
        config: &Path,
        xdg_config: &Path,
        codex_home: &Path,
    ) -> Result<Vec<PathBuf>, HookError> {
        let executable = executable.to_str().ok_or(HookError::InvalidExecutable)?;
        let grouped_providers = [AgentTool::Claude, AgentTool::Codex, AgentTool::Gemini];
        #[cfg(windows)]
        let kimi_command = Self::powershell_hook_command(executable, AgentTool::Kimi);
        #[cfg(not(windows))]
        let kimi_command = Self::posix_hook_command(executable, AgentTool::Kimi)?;
        #[cfg(windows)]
        let copilot_command = Self::powershell_hook_command(executable, AgentTool::Copilot);
        #[cfg(not(windows))]
        let copilot_command = Self::posix_hook_command(executable, AgentTool::Copilot)?;
        let paths = vec![
            home.join(CLAUDE_SETTINGS),
            codex_home.join("hooks.json"),
            home.join(GEMINI_SETTINGS),
            home.join(COPILOT_HOOK),
            home.join(KIMI_CONFIG),
            config.join(AMP_PLUGIN),
            xdg_config.join(OPENCODE_PLUGIN),
        ];
        #[cfg(windows)]
        for (path, provider) in paths[..3].iter().zip(grouped_providers) {
            let command = Self::powershell_hook_command(executable, provider);
            Self::install_grouped_json(path, provider, &command)?;
        }
        #[cfg(not(windows))]
        for (path, provider) in paths[..3].iter().zip(grouped_providers) {
            let command = Self::posix_hook_command(executable, provider)?;
            Self::install_grouped_json(path, provider, &command)?;
        }
        Self::install_copilot(&paths[3], &copilot_command)?;
        Self::install_kimi(&paths[4], AgentTool::Kimi, &kimi_command)?;
        Self::write_text(&paths[5], &Self::amp_plugin(executable)?)?;
        Self::write_text(&paths[6], &Self::opencode_plugin(executable)?)?;
        Ok(paths)
    }

    /// Returns the CLI arguments for one provider's lifecycle callback.
    fn capture_arguments(provider: AgentTool) -> [String; 4] {
        [
            HOOK_SUBCOMMAND.to_string(),
            CAPTURE_SUBCOMMAND.to_string(),
            CAPTURE_PROVIDER_ARGUMENT.to_string(),
            provider.protocol_token().to_string(),
        ]
    }

    /// Builds a POSIX shell command for provider hook formats that accept one
    /// command string.
    ///
    /// # Errors
    /// Returns a [`HookError`] if the executable cannot be safely quoted.
    fn posix_hook_command(executable: &str, provider: AgentTool) -> Result<String, HookError> {
        let executable = shlex::try_quote(executable).map_err(|_| HookError::InvalidExecutable)?;
        Ok(format!(
            "{executable} {} {CAPTURE_PROCESS_ID_ARGUMENT} \"$PPID\" {CAPTURE_PARENT_PROCESS_ID_ARGUMENT} \"$(ps -o ppid= -p \"$PPID\" | tr -d '[:space:]')\"",
            Self::capture_arguments(provider).join(" "),
        ))
    }

    /// Builds a PowerShell invocation, including the call operator required for
    /// a quoted executable path.
    #[cfg(any(windows, test))]
    fn powershell_hook_command(executable: &str, provider: AgentTool) -> String {
        let executable = executable.replace('\'', "''");
        format!(
            "$provider = (Get-CimInstance -ClassName Win32_Process -Filter \"ProcessId=$PID\").ParentProcessId; $parent = (Get-CimInstance -ClassName Win32_Process -Filter \"ProcessId=$provider\").ParentProcessId; & '{executable}' {} {CAPTURE_PROCESS_ID_ARGUMENT} $provider {CAPTURE_PARENT_PROCESS_ID_ARGUMENT} $parent",
            Self::capture_arguments(provider).join(" "),
        )
    }

    /// Reconciles Muster's provider-specific entry in a grouped `SessionStart`
    /// hook array, replacing stale executable paths and removing duplicates.
    ///
    /// # Errors
    /// Returns a [`HookError`] for malformed JSON, incompatible shapes, or I/O.
    fn install_grouped_json(
        path: &Path,
        provider: AgentTool,
        command: &str,
    ) -> Result<(), HookError> {
        let mut root = Self::read_json(path)?;
        let object = root
            .as_object_mut()
            .ok_or_else(|| HookError::Schema(path.to_path_buf()))?;
        let hooks = Self::object_entry(object, "hooks", path)?;
        let entries = Self::array_entry(hooks, SESSION_START_EVENT, path)?;
        let mut installed = false;
        let mut changed = false;
        for entry in entries.iter_mut() {
            let hooks = entry
                .as_object_mut()
                .and_then(|entry| entry.get_mut("hooks"))
                .and_then(Value::as_array_mut)
                .ok_or_else(|| HookError::Schema(path.to_path_buf()))?;
            hooks.retain_mut(|hook| {
                let Some(existing) = hook.get("command").and_then(Value::as_str) else {
                    return true;
                };
                if !Self::is_provider_capture_command(existing, provider) {
                    return true;
                }
                if installed {
                    changed = true;
                    return false;
                }
                installed = true;
                if existing != command {
                    if let Some(object) = hook.as_object_mut() {
                        object.insert("command".to_string(), Value::String(command.to_string()));
                    }
                    changed = true;
                }
                true
            });
        }
        let entry_count = entries.len();
        entries.retain(|entry| !Self::is_empty_owned_hook_group(entry));
        changed |= entries.len() != entry_count;
        if !installed {
            entries.push(json!({
                "hooks": [{ "type": "command", "command": command }]
            }));
            changed = true;
        }
        if changed {
            Self::write_json(path, &root)?;
        }
        Ok(())
    }

    /// Whether `command` invokes Muster's capture receiver for `provider`,
    /// regardless of the executable path installed by an earlier version.
    fn is_provider_capture_command(command: &str, provider: AgentTool) -> bool {
        shlex::split(command).is_some_and(|arguments| {
            arguments
                .as_slice()
                .windows(Self::capture_arguments(provider).len())
                .any(|arguments| arguments == Self::capture_arguments(provider))
        })
    }

    /// Whether an entry became an empty Muster-owned group after duplicate
    /// capture commands were removed.
    fn is_empty_owned_hook_group(entry: &Value) -> bool {
        entry.as_object().is_some_and(|entry| {
            entry.len() == 1
                && entry
                    .get("hooks")
                    .and_then(Value::as_array)
                    .is_some_and(Vec::is_empty)
        })
    }

    /// Writes Copilot's dedicated user hook file.
    ///
    /// # Errors
    /// Returns a [`HookError`] if the owned hook file cannot be written.
    fn install_copilot(path: &Path, command: &str) -> Result<(), HookError> {
        #[cfg(windows)]
        let command_key = "powershell";
        #[cfg(not(windows))]
        let command_key = "bash";
        let hook = json!({
            "version": 1,
            "hooks": {
                COPILOT_SESSION_START_EVENT: [{
                    "type": "command",
                    command_key: command
                }]
            }
        });
        Self::write_json(path, &hook)
    }

    /// Reconciles Kimi's provider-specific `SessionStart` hook while preserving
    /// unrelated TOML and hook entries.
    ///
    /// # Errors
    /// Returns a [`HookError`] for malformed TOML, incompatible shapes, or I/O.
    fn install_kimi(path: &Path, provider: AgentTool, command: &str) -> Result<(), HookError> {
        let raw = Self::read_text(path)?;
        let mut document = if raw.trim().is_empty() {
            DocumentMut::new()
        } else {
            raw.parse::<DocumentMut>()
                .map_err(|source| HookError::Toml {
                    path: path.to_path_buf(),
                    source,
                })?
        };
        if document.get("hooks").is_none() {
            document["hooks"] = Item::ArrayOfTables(ArrayOfTables::new());
        }
        let hooks = document["hooks"]
            .as_array_of_tables_mut()
            .ok_or_else(|| HookError::Schema(path.to_path_buf()))?;
        let mut installed = false;
        let mut changed = false;
        let mut duplicates = Vec::new();
        for (index, hook) in hooks.iter_mut().enumerate() {
            let Some(existing) = hook.get("command").and_then(Item::as_str) else {
                continue;
            };
            if hook.get("event").and_then(Item::as_str) != Some(SESSION_START_EVENT)
                || !Self::is_provider_capture_command(existing, provider)
            {
                continue;
            }
            if installed {
                duplicates.push(index);
                continue;
            }
            installed = true;
            if existing != command {
                hook["command"] = toml_edit::value(command);
                changed = true;
            }
            if hook.get("matcher").and_then(Item::as_str) != Some(KIMI_SESSION_MATCHER) {
                hook["matcher"] = toml_edit::value(KIMI_SESSION_MATCHER);
                changed = true;
            }
        }
        for index in duplicates.into_iter().rev() {
            hooks.remove(index);
            changed = true;
        }
        if !installed {
            let mut hook = Table::new();
            hook["event"] = toml_edit::value(SESSION_START_EVENT);
            hook["matcher"] = toml_edit::value(KIMI_SESSION_MATCHER);
            hook["command"] = toml_edit::value(command);
            hooks.push(hook);
            changed = true;
        }
        if changed {
            Self::write_text(path, &document.to_string())?;
        }
        Ok(())
    }

    /// Returns an Amp plugin that reports `event.thread.id` on session start.
    ///
    /// # Errors
    /// Returns a [`HookError`] if the executable cannot be JSON encoded.
    fn amp_plugin(executable: &str) -> Result<String, HookError> {
        let executable = serde_json::to_string(executable).map_err(HookError::PluginEncoding)?;
        let owner =
            serde_json::to_string(MUSTER_AGENT_SESSION_ENV).map_err(HookError::PluginEncoding)?;
        let arguments = serde_json::to_string(&Self::capture_arguments(AgentTool::Amp))
            .map_err(HookError::PluginEncoding)?;
        Ok(format!(
            r#"import {{ spawn }} from "node:child_process"
import type {{ PluginAPI }} from "@ampcode/plugin"

const executable = {executable}
const active = Boolean(process.env[{owner}])

const capture = (sessionId: string) => {{
  if (!active) return
  const child = spawn(executable, [...{arguments}, "{CAPTURE_PROCESS_ID_ARGUMENT}", process.pid.toString(), "{CAPTURE_PARENT_PROCESS_ID_ARGUMENT}", process.ppid.toString()], {{ stdio: ["pipe", "ignore", "ignore"] }})
  child.on("error", () => {{}})
  child.stdin.on("error", () => {{}})
  child.stdin.end(JSON.stringify({{ version: {AGENT_PROTOCOL_VERSION}, event: "session_started", session_id: sessionId }}))
}}

export default function musterSession(amp: PluginAPI) {{
  amp.on("session.start", (event) => capture(event.thread.id))
}}
"#
        ))
    }

    /// Returns an OpenCode plugin that reports IDs from session lifecycle events.
    ///
    /// # Errors
    /// Returns a [`HookError`] if the executable cannot be JSON encoded.
    fn opencode_plugin(executable: &str) -> Result<String, HookError> {
        let executable = serde_json::to_string(executable).map_err(HookError::PluginEncoding)?;
        let owner =
            serde_json::to_string(MUSTER_AGENT_SESSION_ENV).map_err(HookError::PluginEncoding)?;
        let arguments = serde_json::to_string(&Self::capture_arguments(AgentTool::Opencode))
            .map_err(HookError::PluginEncoding)?;
        Ok(format!(
            r#"import {{ spawn }} from "node:child_process"

const executable = {executable}
const active = Boolean(process.env[{owner}])
const sessionParents = new Map()
let activeSessionId
let capturedSessionId
let pendingSessionId
let captureInFlight = false

const flush = () => {{
  if (!active || captureInFlight || !pendingSessionId || pendingSessionId === capturedSessionId) return
  const sessionId = pendingSessionId
  pendingSessionId = undefined
  captureInFlight = true
  const child = spawn(executable, [...{arguments}, "{CAPTURE_PROCESS_ID_ARGUMENT}", process.pid.toString(), "{CAPTURE_PARENT_PROCESS_ID_ARGUMENT}", process.ppid.toString()], {{ stdio: ["pipe", "ignore", "ignore"] }})
  let settled = false
  const complete = (succeeded) => {{
    if (settled) return
    settled = true
    if (succeeded) capturedSessionId = sessionId
    captureInFlight = false
    flush()
  }}
  child.on("error", () => complete(false))
  child.on("exit", (code) => complete(code === 0))
  child.stdin.on("error", () => complete(false))
  child.stdin.end(JSON.stringify({{ version: {AGENT_PROTOCOL_VERSION}, event: "session_started", session_id: sessionId }}))
}}

const capture = (sessionId) => {{
  if (!active || !sessionId || sessionId === capturedSessionId || sessionId === pendingSessionId) return
  pendingSessionId = sessionId
  flush()
}}

export const MusterSession = async ({{ client }}) => {{
  const known = await client.session.list().catch(() => undefined)
  for (const info of known?.data ?? []) sessionParents.set(info.id, info.parentID)

  const select = (sessionId) => {{
    if (!sessionParents.has(sessionId) || sessionParents.get(sessionId)) return
    activeSessionId = sessionId
    capture(sessionId)
  }}

  return {{
    event: async ({{ event }}) => {{
      if (event.type === "session.created" || event.type === "session.updated") {{
        const info = event.properties?.info ?? event.properties?.session ?? event.properties
        const sessionId = info?.id ?? info?.sessionID
        if (sessionId) sessionParents.set(sessionId, info?.parentID)
        if (sessionId === activeSessionId && !info?.parentID) capture(sessionId)
        return
      }}
      if (event.type === "session.deleted") {{
        const info = event.properties?.info ?? event.properties?.session ?? event.properties
        const sessionId = info?.id ?? info?.sessionID
        if (sessionId) sessionParents.delete(sessionId)
        if (sessionId === activeSessionId) activeSessionId = undefined
        return
      }}
      if (event.type === "tui.session.select") select(event.properties?.sessionID)
    }},
    "chat.message": async (input) => select(input.sessionID),
  }}
}}
"#
        ))
    }

    /// Extracts a provider session ID from the canonical protocol or a native
    /// compatibility payload.
    ///
    /// # Errors
    /// Returns a [`HookError`] for unsupported versions or absent identities.
    fn native_id(payload: &Value) -> Result<NativeSessionId, HookError> {
        if let Some(version) = payload.get("version") {
            let version = version
                .as_u64()
                .ok_or(HookError::UnsupportedProtocolVersion(u64::MAX))?;
            if version != u64::from(AGENT_PROTOCOL_VERSION) {
                return Err(HookError::UnsupportedProtocolVersion(version));
            }
            let event = payload
                .get("event")
                .and_then(Value::as_str)
                .ok_or(HookError::MissingProtocolEvent)?;
            if event != PROTOCOL_SESSION_STARTED {
                return Err(HookError::UnsupportedProtocolEvent(event.to_string()));
            }
        }
        let native = payload
            .get("session_id")
            .or_else(|| payload.get("sessionId"))
            .and_then(Value::as_str)
            .ok_or(HookError::MissingSessionId)?;
        NativeSessionId::try_new(native).map_err(|_| HookError::MissingSessionId)
    }

    /// Reads JSON or returns an empty object when the file does not exist.
    ///
    /// # Errors
    /// Returns a [`HookError`] for I/O or parse failures.
    fn read_json(path: &Path) -> Result<Value, HookError> {
        let raw = Self::read_text(path)?;
        if raw.trim().is_empty() {
            return Ok(Value::Object(Map::new()));
        }
        serde_json::from_str(&raw).map_err(|source| HookError::Json {
            path: path.to_path_buf(),
            source,
        })
    }

    /// Reads UTF-8 text or returns empty text when the file is absent.
    ///
    /// # Errors
    /// Returns a [`HookError`] when an existing file cannot be read.
    fn read_text(path: &Path) -> Result<String, HookError> {
        match fs::read_to_string(path) {
            Ok(raw) => Ok(raw),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
            Err(source) => Err(HookError::Read {
                path: path.to_path_buf(),
                source,
            }),
        }
    }

    /// Returns an object field, creating it only when absent.
    ///
    /// # Errors
    /// Returns a [`HookError`] when an existing field is not an object.
    fn object_entry<'a>(
        parent: &'a mut Map<String, Value>,
        key: &str,
        path: &Path,
    ) -> Result<&'a mut Map<String, Value>, HookError> {
        parent
            .entry(key)
            .or_insert_with(|| Value::Object(Map::new()))
            .as_object_mut()
            .ok_or_else(|| HookError::Schema(path.to_path_buf()))
    }

    /// Returns an array field, creating it only when absent.
    ///
    /// # Errors
    /// Returns a [`HookError`] when an existing field is not an array.
    fn array_entry<'a>(
        parent: &'a mut Map<String, Value>,
        key: &str,
        path: &Path,
    ) -> Result<&'a mut Vec<Value>, HookError> {
        parent
            .entry(key)
            .or_insert_with(|| Value::Array(Vec::new()))
            .as_array_mut()
            .ok_or_else(|| HookError::Schema(path.to_path_buf()))
    }

    /// Serializes formatted JSON and writes it atomically.
    ///
    /// # Errors
    /// Returns a [`HookError`] if serialization or writing fails.
    fn write_json(path: &Path, value: &Value) -> Result<(), HookError> {
        let mut raw = serde_json::to_string_pretty(value).map_err(|source| HookError::Json {
            path: path.to_path_buf(),
            source,
        })?;
        raw.push('\n');
        Self::write_text(path, &raw)
    }

    /// Writes an owned integration file atomically while preserving an existing
    /// target's permissions and any symlink used to reach it.
    ///
    /// # Errors
    /// Returns a [`HookError`] if path resolution, writing, or replacement fails.
    fn write_text(path: &Path, raw: &str) -> Result<(), HookError> {
        let destination = Self::write_destination(path)?;
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).map_err(|source| HookError::Write {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        if fs::read_to_string(&destination).is_ok_and(|current| current == raw) {
            return Ok(());
        }
        let permissions = match fs::metadata(&destination) {
            Ok(metadata) => Some(metadata.permissions()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(source) => {
                return Err(HookError::Read {
                    path: destination,
                    source,
                });
            },
        };
        let mut file = AtomicWriteFile::open(&destination).map_err(|source| HookError::Write {
            path: destination.clone(),
            source,
        })?;
        file.write_all(raw.as_bytes())
            .map_err(|source| HookError::Write {
                path: destination.clone(),
                source,
            })?;
        if let Some(permissions) = permissions {
            file.set_permissions(permissions)
                .map_err(|source| HookError::Write {
                    path: destination.clone(),
                    source,
                })?;
        }
        file.commit().map_err(|source| HookError::Write {
            path: destination,
            source,
        })
    }

    /// Resolves existing symlinks to their target without requiring the final
    /// target to exist, so atomic replacement leaves every alias intact.
    ///
    /// # Errors
    /// Returns a [`HookError`] when metadata or symlink resolution fails.
    fn write_destination(path: &Path) -> Result<PathBuf, HookError> {
        let mut destination = path.to_path_buf();
        for depth in 0..=MAX_PROVIDER_CONFIG_SYMLINKS {
            match fs::symlink_metadata(&destination) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    if depth == MAX_PROVIDER_CONFIG_SYMLINKS {
                        return Err(HookError::SymlinkDepth(destination));
                    }
                    let target = fs::read_link(&destination).map_err(|source| HookError::Read {
                        path: destination.clone(),
                        source,
                    })?;
                    destination = if target.is_absolute() {
                        target
                    } else {
                        match destination.parent() {
                            Some(parent) => parent.join(target),
                            None => target,
                        }
                    };
                },
                Ok(_) => return Ok(destination),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    return Ok(destination);
                },
                Err(source) => {
                    return Err(HookError::Read {
                        path: destination,
                        source,
                    });
                },
            }
        }
        Err(HookError::SymlinkDepth(destination))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Setup is idempotent and preserves unrelated JSON/TOML settings.
    #[test]
    fn setup_preserves_configs_without_duplicating_hooks() {
        let root = std::env::temp_dir().join(format!("muster-hooks-{}", uuid::Uuid::new_v4()));
        let home = root.join("home");
        let config = root.join("config");
        let xdg_config = root.join("xdg");
        let claude = home.join(CLAUDE_SETTINGS);
        let kimi = home.join(KIMI_CONFIG);
        ProviderHooks::write_text(&claude, "{\"theme\":\"dark\"}").unwrap();
        ProviderHooks::write_text(&kimi, "model = \"kimi\"\n").unwrap();

        let paths =
            ProviderHooks::setup_in(Path::new("/opt/muster"), &home, &config, &xdg_config).unwrap();
        ProviderHooks::setup_in(Path::new("/opt/muster"), &home, &config, &xdg_config).unwrap();

        let claude: Value = serde_json::from_str(&fs::read_to_string(claude).unwrap()).unwrap();
        assert_eq!(claude["theme"], "dark");
        assert_eq!(
            claude["hooks"][SESSION_START_EVENT]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        let kimi = fs::read_to_string(kimi).unwrap();
        assert!(kimi.contains("model = \"kimi\""));
        assert_eq!(kimi.matches("event = \"SessionStart\"").count(), 1);
        assert_eq!(paths[6], xdg_config.join(OPENCODE_PLUGIN));
        assert!(paths[6].is_file());
        assert!(!config.join(OPENCODE_PLUGIN).exists());
        fs::remove_dir_all(root).unwrap();
    }

    /// Setup replaces owned grouped and Kimi hooks when the executable moves,
    /// removing duplicate copies without disturbing unrelated commands.
    #[test]
    fn setup_reconciles_outdated_hook_commands() {
        const OLD_EXECUTABLE: &str = "/old/muster";
        const NEW_EXECUTABLE: &str = "/new/muster";
        const UNRELATED_COMMAND: &str = "notify-session";

        let root_path =
            std::env::temp_dir().join(format!("muster-hook-upgrade-{}", uuid::Uuid::new_v4()));
        let home = root_path.join("home");
        let config = root_path.join("config");
        let xdg_config = root_path.join("xdg");
        let claude = home.join(CLAUDE_SETTINGS);
        let kimi = home.join(KIMI_CONFIG);
        let outdated =
            ProviderHooks::posix_hook_command(OLD_EXECUTABLE, AgentTool::Claude).unwrap();
        let current = ProviderHooks::posix_hook_command(NEW_EXECUTABLE, AgentTool::Claude).unwrap();
        let outdated_kimi =
            ProviderHooks::posix_hook_command(OLD_EXECUTABLE, AgentTool::Kimi).unwrap();
        let current_kimi =
            ProviderHooks::posix_hook_command(NEW_EXECUTABLE, AgentTool::Kimi).unwrap();
        ProviderHooks::write_json(
            &claude,
            &json!({
                "hooks": {
                    SESSION_START_EVENT: [
                        {
                            "hooks": [
                                { "type": "command", "command": outdated },
                                { "type": "command", "command": UNRELATED_COMMAND }
                            ]
                        },
                        {
                            "hooks": [
                                { "type": "command", "command": current }
                            ]
                        }
                    ]
                }
            }),
        )
        .unwrap();
        let mut kimi_config = DocumentMut::new();
        kimi_config["hooks"] = Item::ArrayOfTables(ArrayOfTables::new());
        for matcher in ["startup", KIMI_SESSION_MATCHER] {
            let mut hook = Table::new();
            hook["event"] = toml_edit::value(SESSION_START_EVENT);
            hook["matcher"] = toml_edit::value(matcher);
            hook["command"] = toml_edit::value(&outdated_kimi);
            kimi_config["hooks"]
                .as_array_of_tables_mut()
                .unwrap()
                .push(hook);
        }
        ProviderHooks::write_text(&kimi, &kimi_config.to_string()).unwrap();

        ProviderHooks::setup_in(Path::new(NEW_EXECUTABLE), &home, &config, &xdg_config).unwrap();

        let config: Value = serde_json::from_str(&fs::read_to_string(&claude).unwrap()).unwrap();
        let commands = config["hooks"][SESSION_START_EVENT]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|entry| entry.get("hooks").and_then(Value::as_array))
            .flatten()
            .filter_map(|hook| hook.get("command").and_then(Value::as_str))
            .collect::<Vec<_>>();
        assert_eq!(
            commands
                .iter()
                .filter(|command| **command == current.as_str())
                .count(),
            1
        );
        assert_eq!(
            commands
                .iter()
                .filter(|command| **command == UNRELATED_COMMAND)
                .count(),
            1
        );
        assert!(!commands.contains(&outdated.as_str()));
        let kimi = fs::read_to_string(kimi).unwrap();
        assert!(!kimi.contains(&outdated_kimi));
        assert_eq!(kimi.matches(&current_kimi).count(), 1);
        assert_eq!(kimi.matches(KIMI_SESSION_MATCHER).count(), 1);
        fs::remove_dir_all(root_path).unwrap();
    }

    /// Grouped provider hooks fail closed when an existing group has no hook array.
    #[test]
    fn setup_rejects_a_malformed_nested_hook_group() {
        let root =
            std::env::temp_dir().join(format!("muster-hook-schema-{}", uuid::Uuid::new_v4()));
        let path = root.join(CLAUDE_SETTINGS);
        ProviderHooks::write_json(
            &path,
            &json!({
                "hooks": {
                    SESSION_START_EVENT: [{ "hooks": "not-an-array" }]
                }
            }),
        )
        .unwrap();

        let result = ProviderHooks::install_grouped_json(&path, AgentTool::Claude, "muster hook");

        assert!(matches!(result, Err(HookError::Schema(error_path)) if error_path == path));
        fs::remove_dir_all(root).unwrap();
    }

    /// An explicit Codex home receives its hook rather than the default path.
    #[test]
    fn setup_uses_the_configured_codex_home() {
        let root = std::env::temp_dir().join(format!("muster-codex-home-{}", uuid::Uuid::new_v4()));
        let home = root.join("home");
        let config = root.join("config");
        let xdg_config = root.join("xdg");
        let codex_home = root.join("custom-codex");

        let paths = ProviderHooks::setup_in_with_codex(
            Path::new("/tmp/muster"),
            &home,
            &config,
            &xdg_config,
            &codex_home,
        )
        .unwrap();

        assert_eq!(paths[1], codex_home.join("hooks.json"));
        assert!(paths[1].is_file());
        fs::remove_dir_all(root).unwrap();
    }

    /// Canonical events and native camel-case hook payloads share one decoder.
    #[test]
    fn decodes_protocol_and_compatibility_payloads() {
        let protocol = json!({
            "version": AGENT_PROTOCOL_VERSION,
            "event": "session_started",
            "session_id": "native-one"
        });
        let native = json!({ "sessionId": "native-two" });

        assert_eq!(
            ProviderHooks::native_id(&protocol).unwrap().as_ref(),
            "native-one"
        );
        assert_eq!(
            ProviderHooks::native_id(&native).unwrap().as_ref(),
            "native-two"
        );
    }

    /// Versioned payloads reject unknown event names instead of interpreting
    /// unrelated provider data as a session lifecycle event.
    #[test]
    fn rejects_unknown_versioned_protocol_events() {
        let event = json!({
            "version": AGENT_PROTOCOL_VERSION,
            "event": "session_closed",
            "session_id": "native-one"
        });

        assert!(matches!(
            ProviderHooks::native_id(&event),
            Err(HookError::UnsupportedProtocolEvent(name)) if name == "session_closed"
        ));
    }

    /// Quoted Windows paths use PowerShell's call operator and escape embedded
    /// single quotes.
    #[test]
    fn powershell_commands_invoke_quoted_executables() {
        let command = ProviderHooks::powershell_hook_command(
            r"C:\Program Files\Muster's\muster.exe",
            AgentTool::Copilot,
        );

        assert_eq!(
            command,
            r#"$provider = (Get-CimInstance -ClassName Win32_Process -Filter "ProcessId=$PID").ParentProcessId; $parent = (Get-CimInstance -ClassName Win32_Process -Filter "ProcessId=$provider").ParentProcessId; & 'C:\Program Files\Muster''s\muster.exe' hook capture --provider copilot --process-id $provider --parent-process-id $parent"#
        );
    }

    /// Owned Node integrations absorb asynchronous spawn and pipe errors when
    /// the configured Muster executable is no longer available.
    #[test]
    fn generated_plugins_handle_capture_process_errors() {
        let amp = ProviderHooks::amp_plugin("/missing/muster").unwrap();
        let opencode = ProviderHooks::opencode_plugin("/missing/muster").unwrap();

        assert!(amp.contains("if (!active) return"));
        assert!(opencode.contains("let pendingSessionId"));
        assert!(opencode.contains("let captureInFlight = false"));
        assert!(opencode.contains("const sessionParents = new Map()"));
        assert!(opencode.contains("activeSessionId = sessionId"));
        assert!(opencode.contains("sessionId === activeSessionId && !info?.parentID"));
        assert!(opencode.contains(r#""chat.message": async (input) => select(input.sessionID)"#));
        assert!(opencode.contains("sessionParents.get(sessionId)"));
        assert!(!opencode.contains("capture(info?.id"));
        assert!(opencode.contains("pendingSessionId = sessionId"));
        assert!(opencode.contains(r#"child.on("error", () => complete(false))"#));
        assert!(opencode.contains(r#"child.stdin.on("error", () => complete(false))"#));
        assert!(amp.contains(CAPTURE_PROVIDER_ARGUMENT));
        assert!(amp.contains("amp"));
        assert!(opencode.contains(CAPTURE_PROVIDER_ARGUMENT));
        assert!(opencode.contains(AgentTool::Opencode.protocol_token()));
        for plugin in [&amp, &opencode] {
            assert!(plugin.contains(MUSTER_AGENT_SESSION_ENV));
        }
        assert!(amp.contains(r#"child.on("error", () => {})"#));
        assert!(amp.contains(r#"child.stdin.on("error", () => {})"#));
    }

    /// Atomic replacement retains restrictive permissions from an existing
    /// provider settings file.
    #[cfg(unix)]
    #[test]
    fn atomic_writes_preserve_existing_permissions() {
        use std::os::unix::fs::PermissionsExt;

        const PRIVATE_MODE: u32 = 0o600;
        const PERMISSION_MASK: u32 = 0o777;
        let root = std::env::temp_dir().join(format!("muster-hook-mode-{}", uuid::Uuid::new_v4()));
        let path = root.join("settings.json");
        ProviderHooks::write_text(&path, "old").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(PRIVATE_MODE)).unwrap();

        ProviderHooks::write_text(&path, "new").unwrap();

        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & PERMISSION_MASK,
            PRIVATE_MODE
        );
        fs::remove_dir_all(root).unwrap();
    }

    /// Rewriting a symlinked provider config updates its target without
    /// replacing the dotfile-managed alias.
    #[cfg(unix)]
    #[test]
    fn atomic_writes_preserve_provider_config_symlinks() {
        use std::os::unix::fs::symlink;

        let root =
            std::env::temp_dir().join(format!("muster-hook-symlink-{}", uuid::Uuid::new_v4()));
        let target = root.join("managed/settings.json");
        let link = root.join("settings.json");
        ProviderHooks::write_text(&target, "old").unwrap();
        symlink(&target, &link).unwrap();

        ProviderHooks::write_text(&link, "new").unwrap();

        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read_to_string(target).unwrap(), "new");
        fs::remove_dir_all(root).unwrap();
    }

    /// First-time setup follows a relative dangling symlink and creates its
    /// target without replacing the dotfile-managed alias.
    #[cfg(unix)]
    #[test]
    fn atomic_writes_preserve_dangling_provider_config_symlinks() {
        use std::os::unix::fs::symlink;

        const RELATIVE_TARGET: &str = "managed/settings.json";
        let root =
            std::env::temp_dir().join(format!("muster-hook-dangling-{}", uuid::Uuid::new_v4()));
        let target = root.join(RELATIVE_TARGET);
        let link = root.join("settings.json");
        fs::create_dir_all(&root).unwrap();
        symlink(RELATIVE_TARGET, &link).unwrap();

        ProviderHooks::write_text(&link, "new").unwrap();

        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read_to_string(target).unwrap(), "new");
        fs::remove_dir_all(root).unwrap();
    }
}
