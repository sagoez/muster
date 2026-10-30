use std::{io, path::PathBuf};

use bytesize::ByteSize;
use thiserror::Error;

use super::{agent::Agent, mcp_scope::McpScope, server_name::McpServerName};

/// A failure to read, change or write an agent's config.
#[derive(Debug, Error)]
pub enum AgentConfigError {
    /// The agent has no config of that scope.
    #[error("{} has no {scope} scope for MCP servers; {}", agent.title(), scope_hint(*agent))]
    UnsupportedScope {
        /// The agent.
        agent: Agent,
        /// The scope asked for.
        scope: McpScope,
    },
    /// A scope belonging to one project was asked for with none to put it in.
    #[error("the {scope} scope belongs to a project and none was found")]
    NoProject {
        /// The scope asked for.
        scope: McpScope,
    },
    /// No home folder is known to find the user's configs in.
    #[error("no home folder is known, so {}'s user config cannot be found", .0.title())]
    NoHome(Agent),
    /// The muster binary's path cannot be written as text.
    #[error("the muster binary's path {} is not UTF-8, so no config can name it", .0.display())]
    CommandNotUtf8(PathBuf),
    /// The project's folder cannot be written as the key of its entry.
    #[error("the project folder {} is not UTF-8, so Claude Code's config cannot name it", .0.display())]
    ProjectNotUtf8(PathBuf),
    /// The file is not one muster can read safely, so it is left alone.
    #[error("{} cannot be parsed ({reason}); fix it by hand, muster left it untouched", path.display())]
    Unparseable {
        /// The file.
        path: PathBuf,
        /// What is wrong with it.
        reason: String,
    },
    /// The file holds something other than an object at its top level.
    #[error("{} does not hold a JSON object; fix it by hand, muster left it untouched", .0.display())]
    NotAnObject(PathBuf),
    /// Where the entry would go holds something other than a table of them.
    #[error("`{key}` in {} is not an object; fix it by hand, muster left it untouched", path.display())]
    NotATable {
        /// The file.
        path: PathBuf,
        /// The key holding the wrong kind of value.
        key: String,
    },
    /// An object muster would edit holds a key twice, which agents read
    /// differently from one another.
    #[error("`{key}` appears twice in one object of {}; fix it by hand, muster left it untouched", path.display())]
    DuplicateKey {
        /// The file.
        path: PathBuf,
        /// The repeated key.
        key: String,
    },
    /// The path leads to something other than a regular file, such as a
    /// pipe or a device.
    #[error("{} is not a regular file; muster left it untouched", .0.display())]
    NotAFile(PathBuf),
    /// The file is larger than any config muster reads.
    #[error("{} is larger than {limit}; muster left it untouched", path.display())]
    TooLarge {
        /// The file.
        path: PathBuf,
        /// The most a config may hold.
        limit: ByteSize,
    },
    /// A project's config leads out of the project, by a link.
    #[error("{} leads outside the project, to {}; muster left it untouched", path.display(), target.display())]
    OutsideProject {
        /// The file, as the agent names it.
        path: PathBuf,
        /// Where it leads.
        target: PathBuf,
    },
    /// The file is read-only, so muster does not replace it.
    #[error("{} is read-only; make it writable to let muster change it", .0.display())]
    ReadOnly(PathBuf),
    /// Another server is configured under the name.
    #[error(
        "{} already has a different server named {name}; pass --force to replace it, or --name to pick another",
        path.display()
    )]
    Taken {
        /// The file.
        path: PathBuf,
        /// The name.
        name: McpServerName,
    },
    /// The server under the name is not muster's, so it is not removed.
    #[error(
        "the server named {name} in {} is not muster's; pass --force to remove it anyway",
        path.display()
    )]
    NotOurs {
        /// The file.
        path: PathBuf,
        /// The name.
        name: McpServerName,
    },
    /// Something else changed the file while muster edited it.
    #[error("{} changed while muster edited it; nothing was written, run the command again", .0.display())]
    Changed(PathBuf),
    /// The agent kept its config locked for longer than muster waits.
    #[error("{} stayed locked by its agent; nothing was written, run the command again", .0.display())]
    Locked(PathBuf),
    /// The person interrupted the command before any config was written.
    #[error("interrupted; nothing was written")]
    Interrupted,
    /// Reading or writing the file failed.
    #[error("could not {action} {}: {source}", path.display())]
    Io {
        /// What was being done, such as `read`.
        action: &'static str,
        /// The file.
        path: PathBuf,
        /// What failed.
        #[source]
        source: io::Error,
    },
    /// Writing one config failed after others were written.
    #[error("{source}; already written: {}", written_list(written))]
    PartlyWritten {
        /// The configs written before the failure, which hold the change.
        written: Vec<PathBuf>,
        /// The failure.
        #[source]
        source: Box<AgentConfigError>,
    },
}

impl AgentConfigError {
    /// Whether the failure is a refusal over what the file holds or who
    /// holds it.
    pub fn is_conflict(&self) -> bool {
        match self {
            Self::Taken { .. } | Self::NotOurs { .. } | Self::Changed(_) | Self::Locked(_) => true,
            Self::PartlyWritten { source, .. } => source.is_conflict(),
            _ => false,
        }
    }

    /// Whether the failure is the filesystem's.
    pub fn is_io(&self) -> bool {
        match self {
            Self::Io { .. } => true,
            Self::PartlyWritten { source, .. } => source.is_io(),
            _ => false,
        }
    }
}

/// What a person passes for Claude Code's scopes.
const CLAUDE_SCOPES: &str = "pass --scope local, project or user";
/// What a person passes for Codex's scopes.
const CODEX_SCOPES: &str = "pass --scope user or project";
/// What a person passes for Zed's one scope.
const ZED_SCOPES: &str = "it reads MCP servers from its user settings only; pass --scope user";
/// Between the configs a partial write names.
const LIST_SEPARATOR: &str = ", ";

/// The scopes `agent` has, for a refusal of another.
fn scope_hint(agent: Agent) -> &'static str {
    match agent {
        Agent::Claude => CLAUDE_SCOPES,
        Agent::Codex => CODEX_SCOPES,
        Agent::Zed => ZED_SCOPES,
    }
}

/// `written` as one line of paths.
fn written_list(written: &[PathBuf]) -> String {
    written
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join(LIST_SEPARATOR)
}
