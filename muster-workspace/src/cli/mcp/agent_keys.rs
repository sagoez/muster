//! The keys and values of the agents' JSON files muster reads or edits:
//! Claude Code's server entries and both agents' hooks files.

/// The key of a hooks file's hooks object, and of a hook group's handlers.
pub const HOOKS_KEY: &str = "hooks";
/// The event whose hooks run as a session starts, resumes, is cleared or
/// is compacted.
pub const SESSION_START_KEY: &str = "SessionStart";
/// The key of a hook group's source filter.
pub const MATCHER_KEY: &str = "matcher";
/// The key of a handler's kind, and of a server entry's transport.
pub const TYPE_KEY: &str = "type";
/// The key of a handler's command line, and of a server entry's program.
pub const COMMAND_KEY: &str = "command";
/// The key of a handler's time limit, in seconds.
pub const TIMEOUT_KEY: &str = "timeout";
/// The kind of handler that runs a command line.
pub const COMMAND_HANDLER: &str = "command";
/// The key of a server entry's arguments.
pub const ARGS_KEY: &str = "args";
/// The key of a server entry's environment.
pub const ENV_KEY: &str = "env";
/// The transport of a server muster's binary serves over stdio.
pub const STDIO_TYPE: &str = "stdio";
/// The key that exempts a server's tools from Claude Code's tool search, so
/// a session holds them from its start.
pub const ALWAYS_LOAD_KEY: &str = "alwaysLoad";
/// The key of a config's servers, by name.
pub const MCP_SERVERS_KEY: &str = "mcpServers";
/// The key of Claude Code's per-folder config, holding local servers.
pub const PROJECTS_KEY: &str = "projects";
/// The key of the transport in `codex mcp get --json`.
pub const TRANSPORT_KEY: &str = "transport";
